use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use sha2::Digest;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};
use topics_and_questions::{
    Binding, CadencePolicy, Delivery, Evidence, TopicWindowGenerator, TranscriptEvent,
};
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("TOPICS_TEST_DATABASE_URL")
        .expect("TOPICS_TEST_DATABASE_URL must name a disposable Topics-only PostgreSQL database");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect Topics test DB");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate Topics test DB");
    pool
}

async fn reset_db(pool: &PgPool) {
    sqlx::query("TRUNCATE candidate_outbox, question_candidates, generation_jobs, transcript_events CASCADE")
        .execute(pool)
        .await
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn sample_event(
    room_id: Uuid,
    conversation_id: Uuid,
    ordinal: u64,
    spoken_offset_secs: i64,
    start_millis: u64,
    end_millis: u64,
    source_id: &str,
    text: &str,
) -> TranscriptEvent {
    let base_time = Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap();
    let spoken_at = base_time + ChronoDuration::seconds(spoken_offset_secs);
    TranscriptEvent {
        schema_version: 1,
        event_id: Uuid::new_v4(),
        binding: Binding {
            conversation_id,
            source_conversation_id: None,
            room_id,
            session_id: Uuid::new_v4(),
        },
        ingest_ordinal: ordinal,
        received_at: spoken_at,
        source_id: Some(source_id.to_string()),
        source_sequence: Some(ordinal),
        source_utterance_id: Some(ordinal),
        source_spoken_at: Some(spoken_at),
        source_start_millis: Some(start_millis),
        source_end_millis: Some(end_millis),
        text: text.to_string(),
    }
}

async fn insert_event_and_job(pool: &PgPool, event: &TranscriptEvent) {
    let digest = sha2::Sha256::digest(serde_json::to_vec(event).unwrap());
    sqlx::query(
        "INSERT INTO transcript_events(
            event_id, schema_version, conversation_id, room_id, session_id,
            source_conversation_id, ingest_ordinal, received_at, source_id, source_sequence,
            source_utterance_id, source_spoken_at, source_start_millis, source_end_millis,
            text, projection_hash
        ) VALUES ($1, 1, $2, $3, $4, NULL, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(event.event_id)
    .bind(event.binding.conversation_id)
    .bind(event.binding.room_id)
    .bind(event.binding.session_id)
    .bind(event.ingest_ordinal as i64)
    .bind(event.received_at)
    .bind(&event.source_id)
    .bind(event.source_sequence.map(|s| s.to_string()))
    .bind(event.source_utterance_id.map(|id| id as i64))
    .bind(event.source_spoken_at)
    .bind(event.source_start_millis.map(|m| m as i64))
    .bind(event.source_end_millis.map(|m| m as i64))
    .bind(&event.text)
    .bind(digest.as_slice())
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO generation_jobs(event_id) VALUES($1)")
        .bind(event.event_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn cleanup_events(pool: &PgPool, events: &[TranscriptEvent]) {
    let ids: Vec<Uuid> = events.iter().map(|e| e.event_id).collect();
    sqlx::query("DELETE FROM candidate_outbox WHERE candidate_id=ANY($1)")
        .bind(&ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM question_candidates WHERE event_id=ANY($1)")
        .bind(&ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM generation_jobs WHERE event_id=ANY($1)")
        .bind(&ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM transcript_events WHERE event_id=ANY($1)")
        .bind(&ids)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn rolling_window_aggregates_multiple_utterances() {
    let pool = pool().await;
    reset_db(&pool).await;
    let room = Uuid::new_v4();
    let conv = Uuid::new_v4();

    let e1 = sample_event(
        room,
        conv,
        1,
        0,
        0,
        3000,
        "sample-1",
        "Distributed consensus requires quorum intersection.",
    );
    let e2 = sample_event(
        room,
        conv,
        2,
        4,
        4000,
        7000,
        "sample-2",
        "Without majority votes nodes can partition independently.",
    );

    insert_event_and_job(&pool, &e1).await;
    insert_event_and_job(&pool, &e2).await;

    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_cadence_policy(CadencePolicy {
            min_interval_between_questions: Duration::ZERO,
            min_words_per_window: 5,
            max_window_duration: Duration::from_secs(60),
            silence_boundary_threshold: Duration::from_secs(5),
        });

    let window = delivery.load_topic_window(&e2).await.unwrap();
    assert_eq!(window.window_events.len(), 2);
    assert_eq!(
        window.combined_text,
        "Distributed consensus requires quorum intersection. Without majority votes nodes can partition independently."
    );
    assert_eq!(window.start_time, e1.source_spoken_at);
    assert_eq!(window.end_time, e2.source_spoken_at);
    assert_eq!(window.total_words, 12);

    cleanup_events(&pool, &[e1, e2]).await;
    pool.close().await;
}

#[tokio::test]
async fn silence_boundary_splits_rolling_windows() {
    let pool = pool().await;
    reset_db(&pool).await;
    let room = Uuid::new_v4();
    let conv = Uuid::new_v4();

    // e1 ends at 3s; e2 begins at 12s (a 9s gap, exceeding 4s threshold)
    let e1 = sample_event(
        room,
        conv,
        1,
        0,
        0,
        3000,
        "sample-1",
        "Earlier discussion about database transaction boundaries.",
    );
    let e2 = sample_event(
        room,
        conv,
        2,
        12,
        12000,
        16000,
        "sample-2",
        "Now shifting to async messaging reliability.",
    );

    insert_event_and_job(&pool, &e1).await;
    insert_event_and_job(&pool, &e2).await;

    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_cadence_policy(CadencePolicy {
            min_interval_between_questions: Duration::ZERO,
            min_words_per_window: 3,
            max_window_duration: Duration::from_secs(120),
            silence_boundary_threshold: Duration::from_secs(4),
        });

    let window = delivery.load_topic_window(&e2).await.unwrap();
    assert_eq!(
        window.window_events.len(),
        1,
        "Silence gap should isolate e2 into a new topic window"
    );
    assert_eq!(
        window.combined_text,
        "Now shifting to async messaging reliability."
    );
    assert_eq!(window.start_time, e2.source_spoken_at);

    cleanup_events(&pool, &[e1, e2]).await;
    pool.close().await;
}

#[tokio::test]
async fn cooldown_abstention_enforces_question_frequency_limits() {
    let pool = pool().await;
    reset_db(&pool).await;
    let room = Uuid::new_v4();
    let conv = Uuid::new_v4();

    let e1 = sample_event(
        room,
        conv,
        1,
        0,
        0,
        3000,
        "sample-1",
        "First primary thesis on event driven architectures.",
    );
    let e2 = sample_event(
        room,
        conv,
        2,
        1,
        1000,
        4000,
        "sample-2",
        "Immediate follow up sentence right after the first thesis.",
    );

    insert_event_and_job(&pool, &e1).await;
    insert_event_and_job(&pool, &e2).await;

    let policy = CadencePolicy {
        min_interval_between_questions: Duration::from_secs(60),
        min_words_per_window: 1,
        max_window_duration: Duration::from_secs(180),
        silence_boundary_threshold: Duration::from_secs(10),
    };

    let generator = Arc::new(TopicWindowGenerator::new(policy, "window-v1"));
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(generator)
        .with_cadence_policy(policy);

    // First event generates a candidate
    let completed1 = delivery.generate_once().await.unwrap();
    assert_eq!(completed1, 1);

    let candidates =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM question_candidates WHERE room_id=$1")
            .bind(room)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(candidates, 1);

    // Second event arrives during the 60s cooldown; must abstain
    let completed2 = delivery.generate_once().await.unwrap();
    assert_eq!(completed2, 1);

    let candidates_after =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM question_candidates WHERE room_id=$1")
            .bind(room)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        candidates_after, 1,
        "Cooldown must prevent generating duplicate/rapid candidates"
    );

    let e2_job =
        sqlx::query("SELECT status, last_error_code FROM generation_jobs WHERE event_id=$1")
            .bind(e2.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(e2_job.get::<String, _>("status"), "complete");
    assert_eq!(
        e2_job.get::<String, _>("last_error_code"),
        "abstained:cooldown_active"
    );

    cleanup_events(&pool, &[e1, e2]).await;
    pool.close().await;
}

#[tokio::test]
async fn conversational_filler_and_repetition_abstain() {
    let pool = pool().await;
    reset_db(&pool).await;
    let room = Uuid::new_v4();
    let conv = Uuid::new_v4();

    let filler = sample_event(
        room,
        conv,
        1,
        0,
        0,
        2000,
        "sample-1",
        "um uh yeah sure ok thanks",
    );
    let pause = sample_event(room, conv, 2, 2, 2000, 3000, "sample-2", "[pause]");
    let answered = sample_event(
        room,
        conv,
        3,
        4,
        4000,
        7000,
        "sample-3",
        "As mentioned earlier, our replication strategy is raft.",
    );

    insert_event_and_job(&pool, &filler).await;
    insert_event_and_job(&pool, &pause).await;
    insert_event_and_job(&pool, &answered).await;

    let policy = CadencePolicy {
        min_interval_between_questions: Duration::ZERO,
        min_words_per_window: 1,
        max_window_duration: Duration::from_secs(60),
        silence_boundary_threshold: Duration::from_secs(5),
    };
    let generator = Arc::new(TopicWindowGenerator::new(policy, "window-v1"));
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(generator)
        .with_cadence_policy(policy);

    // Process filler
    delivery.generate_once().await.unwrap();
    let job1 = sqlx::query("SELECT status, last_error_code FROM generation_jobs WHERE event_id=$1")
        .bind(filler.event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(job1.get::<String, _>("status"), "complete");
    assert_eq!(
        job1.get::<String, _>("last_error_code"),
        "abstained:low_value_filler"
    );

    // Process pause
    delivery.generate_once().await.unwrap();
    let job2 = sqlx::query("SELECT status, last_error_code FROM generation_jobs WHERE event_id=$1")
        .bind(pause.event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(job2.get::<String, _>("status"), "complete");
    assert_eq!(
        job2.get::<String, _>("last_error_code"),
        "abstained:silence_or_pause"
    );

    // Process already answered
    delivery.generate_once().await.unwrap();
    let job3 = sqlx::query("SELECT status, last_error_code FROM generation_jobs WHERE event_id=$1")
        .bind(answered.event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(job3.get::<String, _>("status"), "complete");
    assert_eq!(
        job3.get::<String, _>("last_error_code"),
        "abstained:already_answered"
    );

    // No candidates produced for non-substantive speech
    let total_candidates =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM question_candidates WHERE room_id=$1")
            .bind(room)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(total_candidates, 0);

    cleanup_events(&pool, &[filler, pause, answered]).await;
    pool.close().await;
}

#[tokio::test]
async fn representative_session_produces_timely_grounded_candidates() {
    let pool = pool().await;
    reset_db(&pool).await;
    let room = Uuid::new_v4();
    let conv = Uuid::new_v4();

    // A realistic sequence representing a conference presentation:
    // 1. Topic 1 introduction
    // 2. Filler acknowledgment
    // 3. Topic 1 elaboration (should produce 1 timely candidate for topic 1)
    // 4. Repetition of statement (abstains)
    // 5. Silence pause (speaker transitions)
    // 6. Topic 2 introduction (should produce 1 candidate for topic 2)
    let e1 = sample_event(
        room,
        conv,
        1,
        0,
        0,
        4000,
        "source-1",
        "We restructured our microservices to use event-driven orchestration.",
    );
    let e2 = sample_event(room, conv, 2, 5, 5000, 6000, "source-2", "um yeah right");
    let e3 = sample_event(
        room,
        conv,
        3,
        7,
        7000,
        11000,
        "source-3",
        "This eliminated distributed transactions and cut p99 latency by half.",
    );
    let e4 = sample_event(
        room,
        conv,
        4,
        12,
        12000,
        16000,
        "source-4",
        "This eliminated distributed transactions and cut p99 latency by half.",
    );
    // 10-second silence gap before Topic 2
    let e5 = sample_event(room, conv, 5, 27, 27000, 28000, "source-5", "[pause]");
    let e6 = sample_event(
        room,
        conv,
        6,
        29,
        29000,
        34000,
        "source-6",
        "Next let us examine how schema evolution is managed with protobuf.",
    );

    let events = vec![e1, e2, e3, e4, e5, e6];
    for e in &events {
        insert_event_and_job(&pool, e).await;
    }

    let policy = CadencePolicy {
        min_interval_between_questions: Duration::from_secs(3),
        min_words_per_window: 5,
        max_window_duration: Duration::from_secs(60),
        silence_boundary_threshold: Duration::from_secs(8),
    };
    let generator = Arc::new(TopicWindowGenerator::new(policy, "window-v1"));
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(generator)
        .with_cadence_policy(policy);

    for _ in 0..events.len() {
        delivery.generate_once().await.unwrap();
    }

    let candidates = sqlx::query(
        "SELECT candidate_id, event_id, text, generator_version, evidence
         FROM question_candidates
         WHERE room_id=$1
         ORDER BY created_at ASC",
    )
    .bind(room)
    .fetch_all(&pool)
    .await
    .unwrap();

    // Out of 6 utterances, exactly 3 substantive topic candidates were generated:
    // e1 (topic 1 start), e3 (topic 1 elaboration after cooldown), and e6 (topic 2 after silence gap).
    // e2 (filler), e4 (repetition), and e5 (pause) were properly filtered out!
    assert_eq!(
        candidates.len(),
        3,
        "Representative session must yield a small, timely set of candidates"
    );
    let candidate_events = candidates
        .iter()
        .map(|candidate| candidate.get::<Uuid, _>("event_id"))
        .collect::<Vec<_>>();
    assert_eq!(
        candidate_events,
        vec![events[0].event_id, events[2].event_id, events[5].event_id]
    );

    let first_evidence: sqlx::types::Json<Evidence> = candidates[0].get("evidence");
    assert_eq!(first_evidence.0.source_id, Some("source-1".into()));
    assert_eq!(first_evidence.0.source_start_millis, Some(0));
    assert_eq!(first_evidence.0.source_end_millis, Some(4000));
    assert!(first_evidence.0.window_start_time.is_some());
    assert!(first_evidence.0.window_end_time.is_some());
    assert!(
        candidates[0]
            .get::<String, _>("text")
            .contains("implication")
    );

    cleanup_events(&pool, &events).await;
    pool.close().await;
}

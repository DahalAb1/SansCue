use chrono::Utc;
use sha2::Digest;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use topics_and_questions::{
    App, Binding, Delivery, GeneratedQuestion, GenerationFailure, GenerationFuture,
    GenerationInput, QuestionGenerator, TranscriptEvent, router,
};
use tower::ServiceExt;
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("TOPICS_TEST_DATABASE_URL").expect("TOPICS_TEST_DATABASE_URL must name a disposable Topics-only PostgreSQL database; this test never skips");
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&url)
        .await
        .expect("connect Topics test DB");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate Topics test DB");
    pool
}

fn event(text: &str) -> TranscriptEvent {
    TranscriptEvent {
        schema_version: 1,
        event_id: Uuid::new_v4(),
        binding: Binding {
            conversation_id: Uuid::new_v4(),
            source_conversation_id: None,
            room_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
        },
        ingest_ordinal: 1,
        received_at: Utc::now(),
        source_id: None,
        source_sequence: None,
        text: text.into(),
    }
}

async fn insert_pending(pool: &PgPool, event: &TranscriptEvent) {
    insert_transcript(pool, event).await;
    sqlx::query("INSERT INTO generation_jobs(event_id) VALUES($1)")
        .bind(event.event_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn insert_transcript(pool: &PgPool, event: &TranscriptEvent) {
    let digest = sha2::Sha256::digest(serde_json::to_vec(event).unwrap());
    sqlx::query("INSERT INTO transcript_events(event_id,schema_version,conversation_id,room_id,session_id,source_conversation_id,ingest_ordinal,received_at,source_id,source_sequence,text,projection_hash) VALUES($1,1,$2,$3,$4,NULL,$5,$6,NULL,NULL,$7,$8)")
        .bind(event.event_id).bind(event.binding.conversation_id).bind(event.binding.room_id).bind(event.binding.session_id).bind(event.ingest_ordinal as i64).bind(event.received_at).bind(&event.text).bind(digest.as_slice()).execute(pool).await.unwrap();
}

async fn cleanup(pool: &PgPool, ids: &[Uuid]) {
    sqlx::query("DELETE FROM candidate_outbox WHERE candidate_id=ANY($1)")
        .bind(ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM question_candidates WHERE event_id=ANY($1)")
        .bind(ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM generation_jobs WHERE event_id=ANY($1)")
        .bind(ids)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM transcript_events WHERE event_id=ANY($1)")
        .bind(ids)
        .execute(pool)
        .await
        .unwrap();
}

#[derive(Default)]
struct FailOnce {
    calls: AtomicUsize,
}
impl QuestionGenerator for FailOnce {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(GenerationFailure {
                    code: "synthetic_temporary_failure",
                    retryable: true,
                });
            }
            Ok(GeneratedQuestion {
                text: format!("Question about {}?", input.event.text),
                generator_version: "fake-v1".into(),
            })
        })
    }
}

struct InvalidOutput;
impl QuestionGenerator for InvalidOutput {
    fn generate<'a>(&'a self, _input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(async {
            Ok(GeneratedQuestion {
                text: String::new(),
                generator_version: "invalid-v1".into(),
            })
        })
    }
}

struct NeverCompletes;
impl QuestionGenerator for NeverCompletes {
    fn generate<'a>(&'a self, _input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn generator_timeout_releases_lease_and_retries_to_exhaustion() {
    let pool = pool().await;
    let event = event("never-completing generator input");
    insert_pending(&pool, &event).await;
    sqlx::query("UPDATE generation_jobs SET max_attempts=2 WHERE event_id=$1")
        .bind(event.event_id)
        .execute(&pool)
        .await
        .unwrap();
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(Arc::new(NeverCompletes))
        .with_generation_deadline(Duration::from_millis(25))
        .unwrap();

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), delivery.generate_once())
            .await
            .unwrap()
            .unwrap(),
        0
    );
    let first = sqlx::query("SELECT status,attempts,next_attempt_at,last_error_code,lease_token FROM generation_jobs WHERE event_id=$1")
        .bind(event.event_id).fetch_one(&pool).await.unwrap();
    assert_eq!(first.get::<String, _>("status"), "pending");
    assert_eq!(first.get::<i32, _>("attempts"), 1);
    assert_eq!(
        first.get::<String, _>("last_error_code"),
        "generation_timeout"
    );
    assert!(first.get::<Option<Uuid>, _>("lease_token").is_none());
    let retry_at: chrono::DateTime<Utc> = first.get("next_attempt_at");
    assert!(retry_at > Utc::now());

    sqlx::query("UPDATE generation_jobs SET next_attempt_at=now() WHERE event_id=$1")
        .bind(event.event_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), delivery.generate_once())
            .await
            .unwrap()
            .unwrap(),
        0
    );
    let exhausted = sqlx::query(
        "SELECT status,attempts,last_error_code,lease_token FROM generation_jobs WHERE event_id=$1",
    )
    .bind(event.event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(exhausted.get::<String, _>("status"), "failed");
    assert_eq!(exhausted.get::<i32, _>("attempts"), 2);
    assert_eq!(
        exhausted.get::<String, _>("last_error_code"),
        "generation_timeout"
    );
    assert!(exhausted.get::<Option<Uuid>, _>("lease_token").is_none());
    cleanup(&pool, &[event.event_id]).await;
    pool.close().await;
}

#[tokio::test]
async fn invalid_generator_output_is_terminal_without_candidate() {
    let pool = pool().await;
    let event = event("invalid output passage");
    insert_pending(&pool, &event).await;
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(Arc::new(InvalidOutput));
    assert_eq!(delivery.generate_once().await.unwrap(), 0);
    let job = sqlx::query(
        "SELECT status,attempts,last_error_code FROM generation_jobs WHERE event_id=$1",
    )
    .bind(event.event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(job.get::<String, _>("status"), "failed");
    assert_eq!(job.get::<i32, _>("attempts"), 1);
    assert_eq!(
        job.get::<String, _>("last_error_code"),
        "invalid_generator_output"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM question_candidates WHERE event_id=$1")
            .bind(event.event_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    cleanup(&pool, &[event.event_id]).await;
    pool.close().await;
}

#[tokio::test]
async fn transient_failure_retries_and_completion_is_atomic() {
    let pool = pool().await;
    let event = event("retry passage");
    insert_pending(&pool, &event).await;
    let generator = Arc::new(FailOnce::default());
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(generator.clone());
    assert_eq!(delivery.generate_once().await.unwrap(), 0);
    let failed = sqlx::query(
        "SELECT status,attempts,last_error_code FROM generation_jobs WHERE event_id=$1",
    )
    .bind(event.event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(failed.get::<String, _>("status"), "pending");
    assert_eq!(failed.get::<i32, _>("attempts"), 1);
    assert_eq!(
        failed.get::<String, _>("last_error_code"),
        "synthetic_temporary_failure"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM question_candidates WHERE event_id=$1")
            .bind(event.event_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );

    sqlx::query("UPDATE generation_jobs SET next_attempt_at=now() WHERE event_id=$1")
        .bind(event.event_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(delivery.generate_once().await.unwrap(), 1);
    let complete =
        sqlx::query("SELECT status,attempts,lease_token FROM generation_jobs WHERE event_id=$1")
            .bind(event.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(complete.get::<String, _>("status"), "complete");
    assert_eq!(complete.get::<i32, _>("attempts"), 2);
    assert!(complete.get::<Option<Uuid>, _>("lease_token").is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM question_candidates WHERE event_id=$1")
            .bind(event.event_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM candidate_outbox o JOIN question_candidates c USING(candidate_id) WHERE c.event_id=$1").bind(event.event_id).fetch_one(&pool).await.unwrap(), 1);
    cleanup(&pool, &[event.event_id]).await;
    pool.close().await;
}

#[derive(Default)]
struct SlowOnce {
    calls: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl QuestionGenerator for SlowOnce {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(async move {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            Ok(GeneratedQuestion {
                text: format!("Question about {}?", input.event.text),
                generator_version: "slow-fake-v1".into(),
            })
        })
    }
}

async fn sessions_ack(
    axum::Json(candidate): axum::Json<topics_and_questions::Candidate>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({"accepted":true,"candidate_id":candidate.candidate_id})),
    )
        .into_response()
}

#[tokio::test]
async fn slow_generation_does_not_block_ingress_and_stale_lease_is_fenced() {
    let pool = pool().await;
    let first = event("slow passage");
    insert_pending(&pool, &first).await;
    let existing = event("already-generated passage");
    insert_transcript(&pool, &existing).await;
    let evidence = topics_and_questions::Evidence {
        conversation_id: existing.binding.conversation_id,
        ingest_ordinal: existing.ingest_ordinal,
        received_at: existing.received_at,
        excerpt: existing.text.clone(),
    };
    sqlx::query("INSERT INTO question_candidates(candidate_id,event_id,room_id,session_id,text,generator_version,evidence) VALUES($1,$1,$2,$3,'Existing candidate?','stub-v1',$4)")
        .bind(existing.event_id).bind(existing.binding.room_id).bind(existing.binding.session_id).bind(sqlx::types::Json(&evidence)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO candidate_outbox(candidate_id) VALUES($1)")
        .bind(existing.event_id)
        .execute(&pool)
        .await
        .unwrap();

    let sessions_calls = Arc::new(AtomicUsize::new(0));
    let calls = sessions_calls.clone();
    let callback = axum::Router::new().route(
        "/internal/v1/topics/candidates",
        axum::routing::post(
            move |candidate: axum::Json<topics_and_questions::Candidate>| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    sessions_ack(candidate).await
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, callback).await.unwrap() });
    let generator = Arc::new(SlowOnce::default());
    let delivery = Delivery::new(pool.clone(), &format!("http://{address}"), "sessions-token")
        .unwrap()
        .with_generator(generator.clone());
    let worker = {
        let delivery = delivery.clone();
        tokio::spawn(async move { delivery.generate_once().await })
    };
    tokio::time::timeout(Duration::from_secs(2), generator.entered.notified())
        .await
        .unwrap();
    assert_eq!(delivery.deliver_once().await.unwrap(), 1);
    assert_eq!(sessions_calls.load(Ordering::SeqCst), 1);

    let second = event("ingress while generator is waiting");
    let app = router(App {
        pool: pool.clone(),
        bee_token: Arc::from("bee-token"),
    });
    let make = |event: &TranscriptEvent| {
        axum::http::Request::builder()
            .method("POST")
            .uri("/internal/v1/transcript-events")
            .header("authorization", "Bearer bee-token")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::to_string(event).unwrap(),
            ))
            .unwrap()
    };
    let response = tokio::time::timeout(Duration::from_secs(2), app.oneshot(make(&second)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::ACCEPTED);
    let second_status: String =
        sqlx::query_scalar("SELECT status FROM generation_jobs WHERE event_id=$1")
            .bind(second.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(second_status, "pending");

    // Simulate lease expiry and a new owner while the first generator call is
    // still in flight. The old completion must not insert candidate/outbox.
    sqlx::query(
        "UPDATE generation_jobs SET lease_until=now()-interval '1 second' WHERE event_id=$1",
    )
    .bind(first.event_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(delivery.generate_once().await.unwrap(), 1);
    generator.release.notify_one();
    assert_eq!(worker.await.unwrap().unwrap(), 0);
    let job = sqlx::query("SELECT status,attempts FROM generation_jobs WHERE event_id=$1")
        .bind(first.event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(job.get::<String, _>("status"), "complete");
    assert_eq!(job.get::<i32, _>("attempts"), 2);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM question_candidates WHERE event_id=$1")
            .bind(first.event_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    server.abort();
    cleanup(&pool, &[first.event_id, second.event_id, existing.event_id]).await;
    pool.close().await;
}

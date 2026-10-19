//! Evaluates the proposed generation policy against a scripted lecture with
//! realistic utterance timing. The generator here is a deterministic stand-in
//! for a model, so these results say nothing about live model quality.
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sha2::Digest;
use sqlx::{Executor, PgPool, Row, postgres::PgConnectOptions, postgres::PgPoolOptions};
use std::{
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use topics_and_questions::{
    AbstentionReason, Binding, CadencePolicy, Delivery, Evidence, GeneratedQuestion,
    GenerationFuture, GenerationInput, GenerationOutcome, QuestionGenerator, TranscriptEvent,
};
use uuid::Uuid;

async fn isolated_pool() -> (PgPool, PgPool, String) {
    let url = std::env::var("TOPICS_TEST_DATABASE_URL")
        .expect("TOPICS_TEST_DATABASE_URL must name a disposable Topics-only PostgreSQL database");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect Topics test DB");
    let schema = format!("policy_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    (pool, admin, schema)
}

async fn finish(pool: PgPool, admin: PgPool, schema: String) {
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    admin.close().await;
}

/// The policy main.rs uses by default, with staleness disabled because the
/// scripted lecture carries fixed historical timestamps.
fn proposed_policy() -> CadencePolicy {
    CadencePolicy {
        min_interval_between_questions: Duration::from_secs(90),
        min_words_per_window: 60,
        max_window_duration: Duration::from_secs(180),
        silence_boundary_threshold: Duration::from_secs(5),
        max_transcript_age: None,
        min_interval_between_attempts: Duration::from_secs(15),
        recent_question_limit: 5,
        recent_question_window: Duration::from_secs(1800),
    }
}

/// Answers like a model that rephrases each time it is asked about a topic.
struct ScriptedModel {
    calls: AtomicUsize,
}

const TOPICS: &[(&str, &[&str])] = &[
    (
        "learning rate",
        &[
            "How clear is why the learning rate controls the step size?",
            "How clear is how the learning rate sets the size of each step?",
        ],
    ),
    (
        "momentum",
        &[
            "How clear is how momentum smooths noisy gradient updates?",
            "How well do you follow why momentum smooths gradient updates?",
        ],
    ),
    (
        "regularization",
        &[
            "How clear is why regularization reduces overfitting?",
            "How clear is how regularization limits overfitting?",
        ],
    ),
];

impl QuestionGenerator for ScriptedModel {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let window = input.window.expect("Delivery always supplies a window");
            for event in window.window_events.iter().rev() {
                let lower = event.text.to_lowercase();
                if let Some((_, phrasings)) = TOPICS.iter().find(|(key, _)| lower.contains(key)) {
                    return Ok(GenerationOutcome::Candidate(GeneratedQuestion {
                        text: phrasings[call % phrasings.len()].to_owned(),
                        evidence_excerpt: event.text.trim().to_owned(),
                        generator_version: "scripted-model/test".into(),
                    }));
                }
            }
            Ok(GenerationOutcome::Abstained {
                reason: AbstentionReason::ModelAbstained,
            })
        })
    }
}

struct Utterance {
    text: &'static str,
    pause_before_secs: f64,
}

fn says(text: &'static str) -> Utterance {
    Utterance {
        text,
        pause_before_secs: 0.6,
    }
}

const LOGISTICS: &[&str] = &[
    "Good morning everyone, and thanks for coming out this early.",
    "Before we start, the slides are posted on the course page.",
    "Please put your phones on silent for the next hour.",
    "Okay, let's get started.",
];
const LEARNING_RATE: &[&str] = &[
    "Today we look at how gradient descent actually moves across a loss surface.",
    "At every step we compute the gradient, which points toward the steepest increase.",
    "We then nudge the weights a little in the opposite direction to reduce the loss.",
    "How far we move on each step is set by one number called the learning rate.",
    "If the learning rate is too large, each step overshoots the minimum and training diverges.",
    "If it is too small, every step is tiny and training takes forever to converge.",
    "So choosing the learning rate is really a balance between speed and stability.",
    "Many teams start with a moderate value and lower it when the loss stops improving.",
];
const MOMENTUM: &[&str] = &[
    "The second idea today is momentum, which changes how each update is computed.",
    "Plain gradient descent only looks at the gradient from the current small batch.",
    "Those batch gradients are noisy, so the path zigzags back and forth across the valley.",
    "Momentum keeps a running average of recent gradients instead of trusting a single one.",
    "You can picture a heavy ball rolling downhill that does not stop at every small bump.",
    "Directions that keep agreeing build up speed, while directions that flip sign cancel out.",
    "In practice we keep about ninety percent of the previous velocity at each step.",
    "That running velocity is added to the weights instead of the raw gradient.",
    "The result is that training moves faster along the valley and wobbles much less.",
    "It also helps the optimizer roll through flat regions where the gradient is tiny.",
    "Most modern optimizers, including Adam, build directly on this momentum idea.",
    "When you tune it, a higher value smooths more but reacts more slowly to change.",
    "If you remember one thing, momentum trades a little reaction time for a smoother path.",
    "We will see the effect on the training curves in the lab this afternoon.",
    "Notice how the momentum curve settles much earlier than the plain one.",
    "That difference matters a lot once the datasets get large.",
];
const TRANSITION: &[&str] = &[
    "For the lab, you will use the same image dataset as last week.",
    "Each group gets a notebook with the training loop already written for you.",
    "Your job is to change one setting at a time and record what happens.",
    "Write down the final loss and how many epochs it took to get there.",
    "We will compare everyone's results together at the end of the session.",
    "If your notebook does not start, raise your hand and a helper will come over.",
    "The cluster queue gets busy after lunch, so start your runs early.",
    "Each run should take about five minutes on the shared machines.",
    "Please save your notebook with your group number in the file name.",
    "We will collect the results sheet before the break at eleven.",
];
const LEARNING_RATE_AGAIN: &[&str] = &[
    "Let me come back to the learning rate for a moment, because it trips people up.",
    "Think of the learning rate as the length of the stride you take downhill.",
    "A huge stride jumps right over the valley, and a tiny one barely moves you.",
    "That is the same trade-off between speed and stability that we saw earlier.",
    "So when the loss explodes, the first thing to try is a smaller learning rate.",
];
const REGULARIZATION: &[&str] = &[
    "Now for something new, which is regularization and why it matters.",
    "A model with enough parameters can memorize the training set perfectly.",
    "That memorized model then does badly on new data, which we call overfitting.",
    "Regularization adds a penalty that grows when the weights become very large.",
    "The optimizer now has to balance fitting the data against keeping weights small.",
    "Smaller weights give smoother functions that tend to generalize better to new examples.",
    "The strength of that penalty is another setting you will tune in the lab.",
    "Too little regularization overfits, and too much makes the model underfit instead.",
];

fn lecture() -> Vec<Utterance> {
    let mut script = Vec::new();
    script.extend(LOGISTICS.iter().map(|text| says(text)));
    script.extend(LEARNING_RATE.iter().map(|text| says(text)));
    script.push(says("Um, okay, right."));
    script.extend(MOMENTUM.iter().map(|text| says(text)));
    script.extend(TRANSITION.iter().map(|text| says(text)));
    script.extend(LEARNING_RATE_AGAIN.iter().map(|text| says(text)));
    let mut after_silence = REGULARIZATION.iter().map(|text| says(text));
    let mut first = after_silence.next().unwrap();
    // The speaker pauses to switch slides; this gap should start a new window.
    first.pause_before_secs = 12.0;
    script.push(first);
    script.extend(after_silence);
    script
}

/// Converts the script into Bee-like events: speech time plus start/end offsets
/// at a typical 2.5 words per second.
fn timed_events(room: Uuid, conversation: Uuid, start: DateTime<Utc>) -> Vec<TranscriptEvent> {
    let session = Uuid::new_v4();
    let mut offset_ms = 0u64;
    lecture()
        .into_iter()
        .enumerate()
        .map(|(index, utterance)| {
            offset_ms += (utterance.pause_before_secs * 1000.0) as u64;
            let words = utterance.text.split_whitespace().count() as u64;
            let duration_ms = words * 400;
            let spoken_at = start + ChronoDuration::milliseconds(offset_ms as i64);
            let event = TranscriptEvent {
                schema_version: 1,
                event_id: Uuid::new_v4(),
                binding: Binding {
                    conversation_id: conversation,
                    source_conversation_id: None,
                    room_id: room,
                    session_id: session,
                },
                ingest_ordinal: index as u64 + 1,
                received_at: spoken_at,
                source_id: Some(format!("sample-{index}")),
                source_sequence: None,
                source_utterance_id: Some(index as u64 + 1),
                source_spoken_at: Some(spoken_at),
                source_start_millis: Some(offset_ms),
                source_end_millis: Some(offset_ms + duration_ms),
                text: utterance.text.to_owned(),
            };
            offset_ms += duration_ms;
            event
        })
        .collect()
}

async fn insert_event_and_job(pool: &PgPool, event: &TranscriptEvent) {
    let digest = sha2::Sha256::digest(serde_json::to_vec(event).unwrap());
    sqlx::query(
        "INSERT INTO transcript_events(
            event_id, schema_version, conversation_id, room_id, session_id,
            source_conversation_id, ingest_ordinal, received_at, source_id, source_sequence,
            source_utterance_id, source_spoken_at, source_start_millis, source_end_millis,
            text, projection_hash
        ) VALUES ($1, 1, $2, $3, $4, NULL, $5, $6, $7, NULL, $8, $9, $10, $11, $12, $13)",
    )
    .bind(event.event_id)
    .bind(event.binding.conversation_id)
    .bind(event.binding.room_id)
    .bind(event.binding.session_id)
    .bind(event.ingest_ordinal as i64)
    .bind(event.received_at)
    .bind(&event.source_id)
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

async fn outcome_codes(pool: &PgPool, events: &[TranscriptEvent]) -> Vec<Option<String>> {
    let mut codes = Vec::new();
    for event in events {
        let row =
            sqlx::query("SELECT status, last_error_code FROM generation_jobs WHERE event_id=$1")
                .bind(event.event_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(row.get::<String, _>("status"), "complete", "{}", event.text);
        codes.push(row.get("last_error_code"));
    }
    codes
}

#[tokio::test]
async fn proposed_policy_yields_bounded_grounded_non_repeating_drafts_for_a_lecture() {
    let (pool, admin, schema) = isolated_pool().await;
    let room = Uuid::new_v4();
    let start = Utc::now() - ChronoDuration::hours(1);
    let events = timed_events(room, Uuid::new_v4(), start);
    for event in &events {
        insert_event_and_job(&pool, event).await;
    }
    let model = Arc::new(ScriptedModel {
        calls: AtomicUsize::new(0),
    });
    let policy = proposed_policy();
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(model.clone())
        .with_cadence_policy(policy);
    for _ in 0..events.len() {
        delivery.generate_once().await.unwrap();
    }

    let drafts = sqlx::query(
        "SELECT event_id, text, evidence FROM question_candidates WHERE room_id=$1 ORDER BY created_at",
    )
    .bind(room)
    .fetch_all(&pool)
    .await
    .unwrap();
    let texts: Vec<String> = drafts.iter().map(|row| row.get("text")).collect();
    // One draft per explained concept; the repeated explanation adds nothing.
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert!(texts[0].contains("learning rate"), "{texts:?}");
    assert!(texts[1].contains("momentum"), "{texts:?}");
    assert!(texts[2].contains("regularization"), "{texts:?}");

    let spoken_at = |id: Uuid| {
        events
            .iter()
            .find(|event| event.event_id == id)
            .and_then(|event| event.source_spoken_at)
            .unwrap()
    };
    let times: Vec<DateTime<Utc>> = drafts
        .iter()
        .map(|row| spoken_at(row.get("event_id")))
        .collect();
    for pair in times.windows(2) {
        assert!(
            pair[1] - pair[0] >= ChronoDuration::seconds(90),
            "drafts must be at least the cooldown apart in speech time"
        );
    }
    for row in &drafts {
        let evidence: sqlx::types::Json<Evidence> = row.get("evidence");
        assert!(
            events
                .iter()
                .any(|event| event.text.contains(&evidence.0.excerpt)),
            "evidence must quote one transcript utterance exactly"
        );
        assert!(
            !LOGISTICS
                .iter()
                .any(|line| evidence.0.excerpt.contains(line))
        );
    }
    let first_draft = (times[0] - start).num_seconds();
    assert!(
        first_draft <= 60,
        "the first explained idea should yield a draft within a minute, took {first_draft}s"
    );

    let codes = outcome_codes(&pool, &events).await;
    let code =
        |text: &str| codes[events.iter().position(|event| event.text == text).unwrap()].clone();
    assert_eq!(
        code("Please put your phones on silent for the next hour."),
        Some("abstained:insufficient_context".into())
    );
    assert!(matches!(
        code("Um, okay, right.").as_deref(),
        Some("abstained:low_value_filler" | "abstained:cooldown_active")
    ));
    // After the slide-change silence the window restarts and must rebuild context.
    assert_eq!(
        code(REGULARIZATION[0]),
        Some("abstained:insufficient_context".into())
    );
    let repeated = LEARNING_RATE_AGAIN
        .iter()
        .chain(TRANSITION)
        .filter(|text| code(text).as_deref() == Some("abstained:repetitive_topic"))
        .count();
    assert!(
        repeated > 0,
        "the repeated explanation must hit duplicate suppression: {codes:?}"
    );

    let calls = model.calls.load(Ordering::SeqCst);
    assert!(calls >= 3, "{calls}");
    assert!(
        calls * 3 <= events.len(),
        "{calls} generator calls for {} utterances; throttling should prevent a call per utterance",
        events.len()
    );
    finish(pool, admin, schema).await;
}

#[tokio::test]
async fn stale_backlog_and_thin_context_never_reach_the_generator() {
    let (pool, admin, schema) = isolated_pool().await;
    let room = Uuid::new_v4();
    // Received two minutes ago and never processed: a backlog, not live speech.
    let backlog = timed_events(
        room,
        Uuid::new_v4(),
        Utc::now() - ChronoDuration::minutes(2),
    );
    let backlog = &backlog[4..12];
    for event in backlog {
        insert_event_and_job(&pool, event).await;
    }
    let model = Arc::new(ScriptedModel {
        calls: AtomicUsize::new(0),
    });
    let delivery = Delivery::new(pool.clone(), "http://127.0.0.1:1", "sessions-token")
        .unwrap()
        .with_generator(model.clone())
        .with_cadence_policy(CadencePolicy {
            max_transcript_age: Some(Duration::from_secs(45)),
            ..proposed_policy()
        });
    for _ in 0..backlog.len() {
        delivery.generate_once().await.unwrap();
    }
    assert!(
        outcome_codes(&pool, backlog)
            .await
            .iter()
            .all(|code| code.as_deref() == Some("abstained:stale_transcript"))
    );

    // Fresh speech that never reaches 60 words of context also costs nothing.
    let other_room = Uuid::new_v4();
    let fresh = timed_events(other_room, Uuid::new_v4(), Utc::now());
    let short = &fresh[4..7];
    for event in short {
        insert_event_and_job(&pool, event).await;
    }
    for _ in 0..short.len() {
        delivery.generate_once().await.unwrap();
    }
    assert!(
        outcome_codes(&pool, short)
            .await
            .iter()
            .all(|code| code.as_deref() == Some("abstained:insufficient_context"))
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    let drafts: i64 = sqlx::query_scalar("SELECT count(*) FROM question_candidates")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(drafts, 0);
    finish(pool, admin, schema).await;
}

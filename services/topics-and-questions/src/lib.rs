use axum::{
    Json, Router,
    extract::Path,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Digest;
use sqlx::{PgPool, Row};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use uuid::Uuid;

pub mod claude;

const DEFAULT_GENERATION_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct App {
    pub pool: PgPool,
    pub bee_token: Arc<str>,
    /// Lets sessions read room transcripts for staff; `None` disables the route.
    pub sessions_token: Option<Arc<str>>,
}
#[derive(Clone)]
pub struct Delivery {
    pub pool: PgPool,
    pub sessions_url: String,
    pub sessions_token: String,
    client: reqwest::Client,
    generator: Arc<dyn QuestionGenerator>,
    generation_deadline: Duration,
    cadence_policy: CadencePolicy,
}

pub type GenerationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<GenerationOutcome, GenerationFailure>> + Send + 'a>>;

/// Provider-neutral generator seam.
pub trait QuestionGenerator: Send + Sync {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicWindow {
    pub room_id: Uuid,
    pub conversation_id: Uuid,
    pub anchor: TranscriptEvent,
    pub window_events: Vec<TranscriptEvent>,
    pub combined_text: String,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub total_words: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CadencePolicy {
    pub min_interval_between_questions: Duration,
    pub min_words_per_window: usize,
    pub max_window_duration: Duration,
    pub silence_boundary_threshold: Duration,
    /// Transcript received longer ago than this is skipped as backlog; `None` disables the check.
    pub max_transcript_age: Option<Duration>,
    /// Minimum speech time between generator calls for one room, so each new
    /// utterance does not trigger another model request.
    pub min_interval_between_attempts: Duration,
    /// Recent drafts (newest first, within the window) used to suppress repeats.
    pub recent_question_limit: usize,
    pub recent_question_window: Duration,
}

impl Default for CadencePolicy {
    fn default() -> Self {
        Self {
            min_interval_between_questions: Duration::ZERO,
            min_words_per_window: 1,
            max_window_duration: Duration::from_secs(180),
            silence_boundary_threshold: Duration::from_secs(5),
            max_transcript_age: None,
            min_interval_between_attempts: Duration::ZERO,
            recent_question_limit: 5,
            recent_question_window: Duration::from_secs(30 * 60),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbstentionReason {
    InsufficientContext,
    CooldownActive {
        remaining_seconds: u64,
    },
    SilenceOrPause,
    RepetitiveTopic,
    AlreadyAnswered,
    LowValueFiller,
    WeaklyGrounded,
    ModelAbstained,
    StaleTranscript,
    /// The newest utterance stops mid-sentence; wait for the explanation to finish.
    UnfinishedThought,
    /// A generator call for this room ran too recently in speech time.
    AttemptThrottled,
    /// Output was not a short comprehension check-in for the Clear / Partly clear / Need help scale.
    UnsuitableQuestion,
}

impl AbstentionReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InsufficientContext => "insufficient_context",
            Self::CooldownActive { .. } => "cooldown_active",
            Self::SilenceOrPause => "silence_or_pause",
            Self::RepetitiveTopic => "repetitive_topic",
            Self::AlreadyAnswered => "already_answered",
            Self::LowValueFiller => "low_value_filler",
            Self::WeaklyGrounded => "weakly_grounded",
            Self::ModelAbstained => "model_abstained",
            Self::StaleTranscript => "stale_transcript",
            Self::UnfinishedThought => "unfinished_thought",
            Self::AttemptThrottled => "attempt_throttled",
            Self::UnsuitableQuestion => "unsuitable_question",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationOutcome {
    Candidate(GeneratedQuestion),
    Abstained { reason: AbstentionReason },
}

impl From<GeneratedQuestion> for GenerationOutcome {
    fn from(q: GeneratedQuestion) -> Self {
        Self::Candidate(q)
    }
}

#[derive(Clone, Debug)]
pub struct GenerationInput {
    pub event: TranscriptEvent,
    pub window: Option<TopicWindow>,
    /// Drafts already produced for this room, newest first.
    pub recent_questions: Vec<RecentQuestion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentQuestion {
    pub text: String,
    pub evidence_excerpt: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedQuestion {
    pub text: String,
    /// Exact contiguous quote from the source event used to ground the question.
    pub evidence_excerpt: String,
    pub generator_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationFailure {
    /// Stable, non-sensitive diagnostic classification persisted for operators.
    pub code: &'static str,
    pub retryable: bool,
}

#[derive(Clone, Default)]
pub struct StubV1Generator;

impl QuestionGenerator for StubV1Generator {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(async move { Ok(stub_question(&input.event).into()) })
    }
}

#[derive(Clone)]
pub struct TopicWindowGenerator {
    pub policy: CadencePolicy,
    pub version: String,
}

impl TopicWindowGenerator {
    pub fn new(policy: CadencePolicy, version: impl Into<String>) -> Self {
        Self {
            policy,
            version: version.into(),
        }
    }
}

/// Deterministic checks that run before any generator call, so silence,
/// filler, thin context and repeats never cost a model request.
pub fn gate(policy: &CadencePolicy, input: &GenerationInput) -> Option<AbstentionReason> {
    let Some(window) = &input.window else {
        return Some(AbstentionReason::InsufficientContext);
    };
    let anchor_text = input.event.text.trim();
    if anchor_text.is_empty() {
        return Some(AbstentionReason::WeaklyGrounded);
    }
    if anchor_text == "..."
        || anchor_text.eq_ignore_ascii_case("[pause]")
        || anchor_text.eq_ignore_ascii_case("[silence]")
        || anchor_text.chars().all(|c| c == '.' || c == ' ')
    {
        return Some(AbstentionReason::SilenceOrPause);
    }
    let lower = anchor_text.to_lowercase();
    let is_filler = lower.split_whitespace().all(|w| {
        matches!(
            w.trim_matches(['.', ',', '!', '?', ';', ':', '-', '"', '\'']),
            "um" | "uh"
                | "yeah"
                | "yep"
                | "yes"
                | "sure"
                | "okay"
                | "ok"
                | "thanks"
                | "thank"
                | "moving"
                | "on"
                | "hello"
                | "hi"
                | "bye"
                | "right"
                | "so"
        )
    });
    if is_filler {
        return Some(AbstentionReason::LowValueFiller);
    }
    if window.total_words < policy.min_words_per_window {
        return Some(AbstentionReason::InsufficientContext);
    }
    let is_repetition = window
        .window_events
        .iter()
        .filter(|e| e.event_id != input.event.event_id)
        .any(|e| e.text.trim().eq_ignore_ascii_case(anchor_text));
    if is_repetition {
        return Some(AbstentionReason::RepetitiveTopic);
    }
    if lower.starts_with("as i said")
        || lower.starts_with("as mentioned")
        || lower.contains("already answered")
        || lower.contains("already covered")
        || lower.contains("like i said earlier")
    {
        return Some(AbstentionReason::AlreadyAnswered);
    }
    if unfinished(anchor_text) {
        return Some(AbstentionReason::UnfinishedThought);
    }
    None
}

/// True when speech visibly stops mid-sentence: a trailing comma, dash or
/// ellipsis, or a dangling connective with no closing punctuation.
fn unfinished(text: &str) -> bool {
    let text = text.trim_end_matches(['"', '\'', ')', ' ']);
    if text.ends_with(',')
        || text.ends_with('-')
        || text.ends_with('\u{2014}')
        || text.ends_with("...")
        || text.ends_with('\u{2026}')
    {
        return true;
    }
    if text.ends_with(['.', '?', '!', ';', ':']) {
        return false;
    }
    let last = text
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or_default()
        .to_lowercase();
    matches!(
        last.as_str(),
        "and"
            | "but"
            | "or"
            | "so"
            | "because"
            | "which"
            | "that"
            | "the"
            | "a"
            | "an"
            | "to"
            | "of"
            | "with"
            | "for"
            | "if"
            | "when"
            | "where"
            | "then"
            | "is"
            | "are"
    )
}

const STOP_WORDS: &[&str] = &[
    "a",
    "about",
    "an",
    "and",
    "are",
    "as",
    "at",
    "be",
    "by",
    "can",
    "clear",
    "do",
    "does",
    "for",
    "from",
    "how",
    "in",
    "into",
    "is",
    "it",
    "its",
    "of",
    "on",
    "or",
    "that",
    "the",
    "their",
    "this",
    "to",
    "understand",
    "understood",
    "was",
    "we",
    "what",
    "when",
    "where",
    "which",
    "why",
    "with",
    "you",
    "your",
    "follow",
    "sense",
    "makes",
    "make",
    "much",
    "well",
];

fn content_words(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .map(|w| match w.strip_suffix('s') {
            Some(singular) if singular.len() > 2 && !singular.ends_with('s') => singular.to_owned(),
            _ => w,
        })
        .filter(|w| w.len() > 1)
        .collect()
}

/// Treats two check-ins as the same idea when most content words of the
/// shorter one also appear in the other, catching light rephrasing.
pub fn near_duplicate(a: &str, b: &str) -> bool {
    let (a, b) = (content_words(a), content_words(b));
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        return false;
    }
    let shared = a.intersection(&b).count();
    shared * 10 >= smaller * 6
}

/// Model output must be a short, single-line clarity check-in that the
/// Clear / Partly clear / Need help scale can answer, not a factual quiz.
pub fn is_check_in(question: &str) -> bool {
    let question = question.trim();
    let lower = question.to_lowercase();
    !question.is_empty()
        && question.ends_with('?')
        && !question.contains(['\n', '\r'])
        && question.split_whitespace().count() <= 25
        && ["clear", "understand", "follow", "sense"]
            .iter()
            .any(|word| lower.contains(word))
}

/// A draft repeats a recent one if its question rephrases it or it quotes the same passage.
pub fn repeats_recent(output: &GeneratedQuestion, recent: &[RecentQuestion]) -> bool {
    recent.iter().any(|previous| {
        near_duplicate(&output.text, &previous.text)
            || output.evidence_excerpt.trim() == previous.evidence_excerpt.trim()
    })
}

impl QuestionGenerator for TopicWindowGenerator {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(async move {
            if let Some(reason) = gate(&self.policy, &input) {
                return Ok(GenerationOutcome::Abstained { reason });
            }
            let anchor_text = input.event.text.trim();
            let excerpt = if anchor_text.len() <= 240 {
                anchor_text.to_string()
            } else {
                anchor_text.chars().take(240).collect::<String>()
            };
            let clean = excerpt.trim_end_matches(['.', '?', '!']);
            let question = GeneratedQuestion {
                text: format!("What is the key implication of: {clean}?"),
                evidence_excerpt: excerpt,
                generator_version: self.version.clone(),
            };

            Ok(GenerationOutcome::Candidate(question))
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptEvent {
    pub schema_version: u16,
    pub event_id: Uuid,
    pub binding: Binding,
    pub ingest_ordinal: u64,
    pub received_at: DateTime<Utc>,
    pub source_id: Option<String>,
    pub source_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_utterance_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_spoken_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_start_millis: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_end_millis: Option<u64>,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Binding {
    pub conversation_id: Uuid,
    pub source_conversation_id: Option<String>,
    pub room_id: Uuid,
    pub session_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Candidate {
    pub candidate_id: Uuid,
    pub event_id: Uuid,
    pub room_id: Uuid,
    pub session_id: Uuid,
    pub text: String,
    pub generator_version: String,
    pub evidence: Evidence,
    pub published: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Evidence {
    pub conversation_id: Uuid,
    pub ingest_ordinal: u64,
    pub received_at: DateTime<Utc>,
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_start_millis: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_end_millis: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end_time: Option<DateTime<Utc>>,
}
#[derive(Clone, Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
}
fn error(status: StatusCode, code: &'static str) -> impl IntoResponse {
    (status, Json(ErrorBody { code }))
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(ready))
        .route("/internal/v1/transcript-events", post(receive_event))
        .route("/internal/v1/rooms/{room}/transcript", get(room_transcript))
        .with_state(app)
}
async fn ready(State(app): State<App>) -> impl IntoResponse {
    if sqlx::query("SELECT 1").execute(&app.pool).await.is_ok() {
        (StatusCode::OK, "ready")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready")
    }
}
fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| !token.is_empty() && constant_eq(v.as_bytes(), token.as_bytes()))
}
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}
/// Lines of a room's received transcript shown to its staff, oldest first.
const TRANSCRIPT_LINES: i64 = 200;

async fn room_transcript(
    State(app): State<App>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> axum::response::Response {
    let Some(token) = app.sessions_token.as_deref() else {
        return error(StatusCode::NOT_FOUND, "not_found").into_response();
    };
    if !authorized(&headers, token) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let Ok(room) = Uuid::parse_str(&room) else {
        return error(StatusCode::NOT_FOUND, "not_found").into_response();
    };
    let rows = sqlx::query(
        "SELECT text, coalesce(source_spoken_at, received_at) AS spoken_at FROM (
             SELECT text, source_spoken_at, received_at, ingest_ordinal FROM transcript_events
             WHERE room_id=$1 ORDER BY coalesce(source_spoken_at, received_at) DESC, ingest_ordinal DESC LIMIT $2
         ) recent ORDER BY coalesce(source_spoken_at, received_at), ingest_ordinal",
    )
    .bind(room)
    .bind(TRANSCRIPT_LINES)
    .fetch_all(&app.pool)
    .await;
    match rows {
        Ok(rows) => Json(json!({
            "lines": rows
                .iter()
                .map(|row| json!({
                    "text": row.get::<String, _>("text"),
                    "spoken_at": row.get::<DateTime<Utc>, _>("spoken_at"),
                }))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "transcript_unavailable").into_response(),
    }
}

async fn receive_event(
    State(app): State<App>,
    headers: HeaderMap,
    Json(event): Json<TranscriptEvent>,
) -> impl IntoResponse {
    if !authorized(&headers, &app.bee_token) {
        return error(StatusCode::UNAUTHORIZED, "service_auth_required").into_response();
    }
    if validate(&event).is_err() {
        return error(StatusCode::BAD_REQUEST, "invalid_transcript_event").into_response();
    }
    let encoded = match serde_json::to_vec(&event) {
        Ok(v) => v,
        Err(_) => {
            return error(StatusCode::BAD_REQUEST, "invalid_transcript_event").into_response();
        }
    };
    let digest = sha2::Sha256::digest(encoded);
    let mut tx = match app.pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return error(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable").into_response();
        }
    };
    let result = sqlx::query(
        "INSERT INTO transcript_events(
            event_id, schema_version, conversation_id, room_id, session_id,
            source_conversation_id, ingest_ordinal, received_at, source_id, source_sequence,
            source_utterance_id, source_spoken_at, source_start_millis, source_end_millis,
            text, projection_hash
        ) VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        ON CONFLICT(event_id) DO NOTHING",
    )
    .bind(event.event_id)
    .bind(event.binding.conversation_id)
    .bind(event.binding.room_id)
    .bind(event.binding.session_id)
    .bind(&event.binding.source_conversation_id)
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
    .execute(&mut *tx)
    .await;
    if result.is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable").into_response();
    }
    let rows = sqlx::query("SELECT projection_hash FROM transcript_events WHERE event_id=$1")
        .bind(event.event_id)
        .fetch_one(&mut *tx)
        .await;
    match rows {
        Ok(row) => {
            let old: Vec<u8> = row.get("projection_hash");
            if old.as_slice() != digest.as_slice() {
                return error(StatusCode::CONFLICT, "event_identity_conflict").into_response();
            }
        }
        Err(_) => {
            return error(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable").into_response();
        }
    }
    if sqlx::query("INSERT INTO generation_jobs(event_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(event.event_id)
        .execute(&mut *tx)
        .await
        .is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable").into_response();
    }
    if tx.commit().await.is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable").into_response();
    }
    (
        StatusCode::ACCEPTED,
        Json(json!({"accepted":true,"event_id":event.event_id})),
    )
        .into_response()
}
pub fn validate(e: &TranscriptEvent) -> Result<(), &'static str> {
    if e.schema_version != 1
        || e.event_id.is_nil()
        || e.binding.conversation_id.is_nil()
        || e.binding.room_id.is_nil()
        || e.binding.session_id.is_nil()
        || e.ingest_ordinal == 0
        || e.ingest_ordinal > i64::MAX as u64
        || e.text.trim().is_empty()
        || e.text.len() > 16_000
        || e.binding
            .source_conversation_id
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 256)
        || e.source_id
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 512)
        || e.source_utterance_id
            .is_some_and(|value| value > i64::MAX as u64)
        || e.source_start_millis
            .is_some_and(|value| value > i64::MAX as u64)
        || e.source_end_millis
            .is_some_and(|value| value > i64::MAX as u64)
        || matches!(
            (e.source_start_millis, e.source_end_millis),
            (Some(start), Some(end)) if end <= start
        )
        || e.source_start_millis.is_some() != e.source_end_millis.is_some()
    {
        Err("invalid event")
    } else {
        Ok(())
    }
}
/// Deliberately transparent deterministic fallback, not a model or production question generator.
pub fn generate(e: &TranscriptEvent) -> Candidate {
    candidate_from(e, stub_question(e), None)
}

fn stub_question(event: &TranscriptEvent) -> GeneratedQuestion {
    let excerpt = event.text.trim().chars().take(240).collect::<String>();
    let clean = excerpt.trim_end_matches(['.', '?', '!']);
    GeneratedQuestion {
        text: format!("What is the key implication of: {clean}?"),
        evidence_excerpt: excerpt,
        generator_version: "stub-v1".into(),
    }
}

fn candidate_from(
    e: &TranscriptEvent,
    output: GeneratedQuestion,
    window: Option<&TopicWindow>,
) -> Candidate {
    let evidence_source = if e.text.contains(&output.evidence_excerpt) {
        e
    } else {
        window
            .and_then(|window| {
                window
                    .window_events
                    .iter()
                    .find(|event| event.text.contains(&output.evidence_excerpt))
            })
            .unwrap_or(e)
    };
    let window_start_time = window
        .and_then(|w| w.start_time)
        .or(e.source_spoken_at)
        .or(Some(e.received_at));
    let window_end_time = window
        .and_then(|w| w.end_time)
        .or(e.source_spoken_at)
        .or(Some(e.received_at));
    Candidate {
        candidate_id: e.event_id,
        event_id: e.event_id,
        room_id: e.binding.room_id,
        session_id: e.binding.session_id,
        text: output.text.trim().to_owned(),
        generator_version: output.generator_version,
        evidence: Evidence {
            conversation_id: evidence_source.binding.conversation_id,
            ingest_ordinal: evidence_source.ingest_ordinal,
            received_at: evidence_source.received_at,
            excerpt: output.evidence_excerpt,
            source_id: evidence_source.source_id.clone(),
            source_start_millis: evidence_source.source_start_millis,
            source_end_millis: evidence_source.source_end_millis,
            window_start_time,
            window_end_time,
        },
        published: false,
    }
}

fn valid_generated_question(
    event: &TranscriptEvent,
    output: &GeneratedQuestion,
    window: Option<&TopicWindow>,
) -> bool {
    let question = output.text.trim();
    let evidence = output.evidence_excerpt.trim();
    let text_grounded = event.text.contains(evidence)
        || window.is_some_and(|window| {
            window
                .window_events
                .iter()
                .any(|source| source.text.contains(evidence))
        });
    !question.is_empty()
        && question.len() <= 1000
        && question.ends_with('?')
        && !question.chars().any(char::is_control)
        && !evidence.is_empty()
        && evidence.len() <= 1000
        && evidence == output.evidence_excerpt
        && text_grounded
        && !output.generator_version.trim().is_empty()
        && output.generator_version.len() <= 128
        && !output.generator_version.chars().any(char::is_control)
}
impl Delivery {
    pub fn new(
        pool: PgPool,
        sessions_url: &str,
        sessions_token: &str,
    ) -> Result<Self, &'static str> {
        let url = reqwest::Url::parse(sessions_url).map_err(|_| "invalid sessions internal URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || sessions_token.trim().is_empty()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("invalid sessions delivery configuration");
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| "HTTP client initialization failed")?;
        Ok(Self {
            pool,
            sessions_url: sessions_url.trim_end_matches('/').to_owned(),
            sessions_token: sessions_token.to_owned(),
            client,
            generator: Arc::new(StubV1Generator),
            generation_deadline: DEFAULT_GENERATION_DEADLINE,
            cadence_policy: CadencePolicy::default(),
        })
    }

    pub fn with_generator(mut self, generator: Arc<dyn QuestionGenerator>) -> Self {
        self.generator = generator;
        self
    }

    pub fn with_cadence_policy(mut self, policy: CadencePolicy) -> Self {
        self.cadence_policy = policy;
        self
    }

    pub fn cadence_policy(&self) -> &CadencePolicy {
        &self.cadence_policy
    }

    pub fn with_generation_deadline(mut self, deadline: Duration) -> Result<Self, &'static str> {
        if deadline.is_zero() {
            return Err("generation deadline must be positive");
        }
        self.generation_deadline = deadline;
        Ok(self)
    }

    pub async fn run(self) -> Result<(), &'static str> {
        let generator_worker = self.clone();
        tokio::spawn(async move {
            loop {
                // Drain queued speech without pausing, so a burst of utterances
                // is not left to age into stale backlog; pause only when idle.
                match generator_worker.generate_pending().await {
                    Ok(completed) if completed > 0 => continue,
                    Ok(_) => {}
                    Err(error) => eprintln!(
                        "topics-and-questions generation worker failed; retrying ({error})"
                    ),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        loop {
            if let Err(error) = self.deliver_once().await {
                eprintln!("topics-and-questions candidate delivery failed; retrying ({error})");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    pub async fn poll_once(&self) -> Result<usize, &'static str> {
        let generated = self.generate_once().await?;
        let delivered = self.deliver_once().await?;
        Ok(generated + delivered)
    }

    pub async fn generate_once(&self) -> Result<usize, &'static str> {
        self.generate_pending().await
    }

    pub async fn deliver_once(&self) -> Result<usize, &'static str> {
        let mut count = 0;
        let outbox=sqlx::query("SELECT c.candidate_id,c.event_id,c.room_id,c.session_id,c.text,c.generator_version,c.evidence FROM candidate_outbox o JOIN question_candidates c USING(candidate_id) WHERE o.delivered_at IS NULL AND o.terminal_at IS NULL ORDER BY o.created_at LIMIT 50").fetch_all(&self.pool).await.map_err(|_|"candidate outbox poll failed")?;
        let mut retryable_failure = false;
        for row in outbox {
            let evidence: sqlx::types::Json<Evidence> = row.get("evidence");
            let candidate = Candidate {
                candidate_id: row.get("candidate_id"),
                event_id: row.get("event_id"),
                room_id: row.get("room_id"),
                session_id: row.get("session_id"),
                text: row.get("text"),
                generator_version: row.get("generator_version"),
                evidence: evidence.0,
                published: false,
            };
            let response = self
                .client
                .post(format!(
                    "{}/internal/v1/topics/candidates",
                    self.sessions_url.trim_end_matches('/')
                ))
                .bearer_auth(&self.sessions_token)
                .json(&candidate)
                .send()
                .await;
            let response = match response {
                Ok(response) => response,
                Err(_) => {
                    retryable_failure = true;
                    continue;
                }
            };
            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body: serde_json::Value =
                    response.json().await.unwrap_or(serde_json::Value::Null);
                if let Some(code) = terminal_candidate_rejection(status, &body) {
                    self.reject_candidate(candidate.candidate_id, code).await?;
                    count += 1;
                } else {
                    retryable_failure = true;
                }
                continue;
            };
            let acknowledgement: serde_json::Value =
                response.json().await.unwrap_or(serde_json::Value::Null);
            if acknowledgement["accepted"] != true
                || acknowledgement["candidate_id"]
                    .as_str()
                    .and_then(|value| Uuid::parse_str(value).ok())
                    != Some(candidate.candidate_id)
            {
                retryable_failure = true;
                continue;
            }
            if sqlx::query("UPDATE candidate_outbox SET delivered_at=now(),attempts=attempts+1 WHERE candidate_id=$1 AND delivered_at IS NULL AND terminal_at IS NULL").bind(candidate.candidate_id).execute(&self.pool).await.is_err() {
                retryable_failure = true;
                continue;
            }
            count += 1;
        }
        if retryable_failure {
            Err("some candidate deliveries remain pending for retry")
        } else {
            Ok(count)
        }
    }

    async fn generate_pending(&self) -> Result<usize, &'static str> {
        // Keep only one lease in flight in this worker. This avoids claiming
        // jobs whose lease would age while they wait behind a slow generator.
        let jobs = self.claim_generation_jobs(1).await?;
        let mut completed = 0;
        for (event, lease) in jobs {
            let early = if self.is_stale(&event) {
                Some(AbstentionReason::StaleTranscript)
            } else {
                self.check_cooldown(&event)
                    .await?
                    .map(|remaining_seconds| AbstentionReason::CooldownActive { remaining_seconds })
            };
            if let Some(reason) = early {
                if self
                    .complete_abstention(event.event_id, &lease, reason.code())
                    .await?
                {
                    completed += 1;
                }
                continue;
            }

            let window = match self.load_topic_window(&event).await {
                Ok(window) => Some(window),
                Err(_) => {
                    self.fail_generation(
                        event.event_id,
                        lease,
                        GenerationFailure {
                            code: "topic_window_unavailable",
                            retryable: true,
                        },
                    )
                    .await?;
                    continue;
                }
            };
            let recent_questions = self.recent_questions(&event).await?;
            let input = GenerationInput {
                event: event.clone(),
                window: window.clone(),
                recent_questions: recent_questions.clone(),
            };
            let gated = match gate(&self.cadence_policy, &input) {
                Some(reason) => Some(reason),
                None if !self.claim_attempt(&event).await? => {
                    Some(AbstentionReason::AttemptThrottled)
                }
                None => None,
            };
            if let Some(reason) = gated {
                if self
                    .complete_abstention(event.event_id, &lease, reason.code())
                    .await?
                {
                    completed += 1;
                }
                continue;
            }
            let (heartbeat, stop_heartbeat) = self.start_lease_heartbeat(event.event_id, lease);
            let result =
                tokio::time::timeout(self.generation_deadline, self.generator.generate(input))
                    .await;
            // Gracefully stop and join the heartbeat before clearing the lease.
            // This prevents an in-flight extension from racing the retry update.
            let _ = stop_heartbeat.send(true);
            let _ = heartbeat.await;
            let result = match result {
                Ok(Ok(GenerationOutcome::Candidate(output)))
                    if repeats_recent(&output, &recent_questions) =>
                {
                    Ok(Ok(GenerationOutcome::Abstained {
                        reason: AbstentionReason::RepetitiveTopic,
                    }))
                }
                other => other,
            };
            match result {
                Ok(Ok(GenerationOutcome::Candidate(output))) => {
                    if valid_generated_question(&event, &output, window.as_ref()) {
                        if self
                            .complete_generation(&event, &lease, output, window.as_ref())
                            .await?
                        {
                            completed += 1;
                        }
                    } else {
                        self.fail_generation(
                            event.event_id,
                            lease,
                            GenerationFailure {
                                code: "invalid_generator_output",
                                retryable: false,
                            },
                        )
                        .await?
                    }
                }
                Ok(Ok(GenerationOutcome::Abstained { reason })) => {
                    if self
                        .complete_abstention(event.event_id, &lease, reason.code())
                        .await?
                    {
                        completed += 1;
                    }
                }
                Ok(Err(failure)) => self.fail_generation(event.event_id, lease, failure).await?,
                Err(_) => {
                    self.fail_generation(
                        event.event_id,
                        lease,
                        GenerationFailure {
                            code: "generation_timeout",
                            retryable: true,
                        },
                    )
                    .await?
                }
            }
        }
        Ok(completed)
    }

    async fn claim_generation_jobs(
        &self,
        limit: i64,
    ) -> Result<Vec<(TranscriptEvent, Uuid)>, &'static str> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| "generation claim transaction failed")?;
        sqlx::query("UPDATE generation_jobs SET status='failed',lease_token=NULL,lease_until=NULL,last_error_code='retry_exhausted',updated_at=now() WHERE status='processing' AND lease_until <= now() AND attempts >= max_attempts")
            .execute(&mut *tx).await.map_err(|_| "generation exhaustion update failed")?;
        let rows = sqlx::query("SELECT j.event_id FROM generation_jobs j JOIN transcript_events e USING(event_id) WHERE j.attempts < j.max_attempts AND ((j.status='pending' AND j.next_attempt_at <= now()) OR (j.status='processing' AND j.lease_until <= now())) ORDER BY e.created_at,j.event_id FOR UPDATE OF j SKIP LOCKED LIMIT $1")
            .bind(limit).fetch_all(&mut *tx).await.map_err(|_| "generation claim query failed")?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row.get("event_id");
            let lease = Uuid::new_v4();
            let updated = sqlx::query("UPDATE generation_jobs SET status='processing',attempts=attempts+1,lease_token=$2,lease_until=now()+interval '60 seconds',updated_at=now() WHERE event_id=$1 RETURNING event_id")
                .bind(id).bind(lease).fetch_optional(&mut *tx).await.map_err(|_| "generation lease update failed")?;
            if updated.is_none() {
                continue;
            }
            let event_row = sqlx::query("SELECT schema_version,event_id,conversation_id,room_id,session_id,source_conversation_id,source_id,source_sequence,source_utterance_id,source_spoken_at,source_start_millis,source_end_millis,ingest_ordinal,received_at,text FROM transcript_events WHERE event_id=$1")
                .bind(id).fetch_one(&mut *tx).await.map_err(|_| "generation input load failed")?;
            claimed.push((
                TranscriptEvent {
                    schema_version: event_row.get::<i16, _>("schema_version") as u16,
                    event_id: event_row.get("event_id"),
                    binding: Binding {
                        conversation_id: event_row.get("conversation_id"),
                        source_conversation_id: event_row.get("source_conversation_id"),
                        room_id: event_row.get("room_id"),
                        session_id: event_row.get("session_id"),
                    },
                    ingest_ordinal: event_row.get::<i64, _>("ingest_ordinal") as u64,
                    received_at: event_row.get("received_at"),
                    source_id: event_row.get("source_id"),
                    source_sequence: event_row
                        .get::<Option<String>, _>("source_sequence")
                        .and_then(|v| v.parse().ok()),
                    source_utterance_id: event_row
                        .get::<Option<i64>, _>("source_utterance_id")
                        .map(|v| v as u64),
                    source_spoken_at: event_row.get("source_spoken_at"),
                    source_start_millis: event_row
                        .get::<Option<i64>, _>("source_start_millis")
                        .map(|v| v as u64),
                    source_end_millis: event_row
                        .get::<Option<i64>, _>("source_end_millis")
                        .map(|v| v as u64),
                    text: event_row.get("text"),
                },
                lease,
            ));
        }
        tx.commit()
            .await
            .map_err(|_| "generation claim commit failed")?;
        Ok(claimed)
    }

    fn start_lease_heartbeat(
        &self,
        event_id: Uuid,
        lease: Uuid,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::sync::watch::Sender<bool>,
    ) {
        let pool = self.pool.clone();
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            interval.tick().await;
            loop {
                tokio::select! {
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() { return; }
                    }
                    _ = interval.tick() => {}
                }
                if *stopped.borrow() {
                    return;
                }
                let result = sqlx::query("UPDATE generation_jobs SET lease_until=now()+interval '60 seconds',updated_at=now() WHERE event_id=$1 AND status='processing' AND lease_token=$2")
                    .bind(event_id).bind(lease).execute(&pool).await;
                if !matches!(result, Ok(ref result) if result.rows_affected() == 1) {
                    return;
                }
            }
        });
        (task, stop)
    }

    fn is_stale(&self, event: &TranscriptEvent) -> bool {
        self.cadence_policy.max_transcript_age.is_some_and(|max| {
            (Utc::now() - event.received_at)
                .to_std()
                .is_ok_and(|age| age > max)
        })
    }

    /// Drafts already produced for this room, newest first, for duplicate suppression.
    pub async fn recent_questions(
        &self,
        anchor: &TranscriptEvent,
    ) -> Result<Vec<RecentQuestion>, &'static str> {
        let policy = &self.cadence_policy;
        if policy.recent_question_limit == 0 {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT text, evidence->>'excerpt' AS excerpt FROM question_candidates
             WHERE room_id=$1 AND created_at > now() - make_interval(secs => $2)
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(anchor.binding.room_id)
        .bind(policy.recent_question_window.as_secs_f64())
        .bind(policy.recent_question_limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| "recent question lookup failed")?;
        Ok(rows
            .into_iter()
            .map(|row| RecentQuestion {
                text: row.get("text"),
                evidence_excerpt: row.get::<Option<String>, _>("excerpt").unwrap_or_default(),
            })
            .collect())
    }

    /// Records a generator attempt for the anchor's room unless one ran within
    /// the configured interval of speech time. Returns false when throttled.
    pub async fn claim_attempt(&self, anchor: &TranscriptEvent) -> Result<bool, &'static str> {
        let interval = self.cadence_policy.min_interval_between_attempts;
        if interval.is_zero() {
            return Ok(true);
        }
        let claimed = sqlx::query(
            "INSERT INTO room_generation_attempts(room_id, last_attempt_at) VALUES($1, $2)
             ON CONFLICT(room_id) DO UPDATE SET last_attempt_at=EXCLUDED.last_attempt_at, updated_at=now()
             WHERE room_generation_attempts.last_attempt_at <= EXCLUDED.last_attempt_at - make_interval(secs => $3)
             RETURNING room_id",
        )
        .bind(anchor.binding.room_id)
        .bind(anchor.source_spoken_at.unwrap_or(anchor.received_at))
        .bind(interval.as_secs_f64())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| "generation attempt claim failed")?;
        Ok(claimed.is_some())
    }

    pub async fn check_cooldown(
        &self,
        anchor: &TranscriptEvent,
    ) -> Result<Option<u64>, &'static str> {
        if self.cadence_policy.min_interval_between_questions.is_zero() {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT created_at, evidence FROM question_candidates
             WHERE room_id=$1
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(anchor.binding.room_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| "cooldown check query failed")?;

        let Some(row) = row else {
            return Ok(None);
        };

        let created_at: DateTime<Utc> = row.get("created_at");
        let evidence: sqlx::types::Json<Evidence> = row.get("evidence");
        let candidate_time = evidence
            .0
            .window_end_time
            .or(evidence.0.window_start_time)
            .unwrap_or(created_at);

        let anchor_time = anchor.source_spoken_at.unwrap_or(anchor.received_at);
        let min_interval = self.cadence_policy.min_interval_between_questions;

        let elapsed = if anchor_time >= candidate_time {
            match (anchor_time - candidate_time).to_std() {
                Ok(d) => d,
                Err(_) => Duration::ZERO,
            }
        } else {
            Duration::ZERO
        };

        if elapsed < min_interval {
            let remaining = (min_interval - elapsed).as_secs();
            Ok(Some(remaining.max(1)))
        } else {
            Ok(None)
        }
    }

    pub async fn load_topic_window(
        &self,
        anchor: &TranscriptEvent,
    ) -> Result<TopicWindow, &'static str> {
        let rows = sqlx::query(
            "SELECT schema_version, event_id, conversation_id, room_id, session_id,
                    source_conversation_id, source_id, source_sequence,
                    source_utterance_id, source_spoken_at, source_start_millis, source_end_millis,
                    ingest_ordinal, received_at, text
             FROM transcript_events
             WHERE room_id = $1 AND conversation_id = $2 AND ingest_ordinal <= $3
             ORDER BY ingest_ordinal DESC
             LIMIT 50",
        )
        .bind(anchor.binding.room_id)
        .bind(anchor.binding.conversation_id)
        .bind(anchor.ingest_ordinal as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| "load topic window events failed")?;

        let mut events: Vec<TranscriptEvent> = rows
            .into_iter()
            .map(|r| TranscriptEvent {
                schema_version: r.get::<i16, _>("schema_version") as u16,
                event_id: r.get("event_id"),
                binding: Binding {
                    conversation_id: r.get("conversation_id"),
                    source_conversation_id: r.get("source_conversation_id"),
                    room_id: r.get("room_id"),
                    session_id: r.get("session_id"),
                },
                ingest_ordinal: r.get::<i64, _>("ingest_ordinal") as u64,
                received_at: r.get("received_at"),
                source_id: r.get("source_id"),
                source_sequence: r
                    .get::<Option<String>, _>("source_sequence")
                    .and_then(|v| v.parse().ok()),
                source_utterance_id: r
                    .get::<Option<i64>, _>("source_utterance_id")
                    .map(|v| v as u64),
                source_spoken_at: r.get("source_spoken_at"),
                source_start_millis: r
                    .get::<Option<i64>, _>("source_start_millis")
                    .map(|v| v as u64),
                source_end_millis: r
                    .get::<Option<i64>, _>("source_end_millis")
                    .map(|v| v as u64),
                text: r.get("text"),
            })
            .collect();

        events.sort_by(|a, b| match (a.source_spoken_at, b.source_spoken_at) {
            (Some(sa), Some(sb)) if sa != sb => sa.cmp(&sb),
            _ => match (a.source_start_millis, b.source_start_millis) {
                (Some(ma), Some(mb)) if ma != mb => ma.cmp(&mb),
                _ => match (a.source_end_millis, b.source_end_millis) {
                    (Some(ea), Some(eb)) if ea != eb => ea.cmp(&eb),
                    _ => match (a.source_sequence, b.source_sequence) {
                        (Some(qa), Some(qb)) if qa != qb => qa.cmp(&qb),
                        _ => a.ingest_ordinal.cmp(&b.ingest_ordinal),
                    },
                },
            },
        });

        let anchor_idx = events
            .iter()
            .position(|e| e.event_id == anchor.event_id)
            .unwrap_or(events.len().saturating_sub(1));

        let anchor_event = &events[anchor_idx];
        let anchor_time = anchor_event
            .source_spoken_at
            .unwrap_or(anchor_event.received_at);

        let max_duration = chrono::Duration::from_std(self.cadence_policy.max_window_duration)
            .unwrap_or(chrono::Duration::MAX);
        let silence_threshold =
            chrono::Duration::from_std(self.cadence_policy.silence_boundary_threshold)
                .unwrap_or(chrono::Duration::MAX);
        let silence_threshold_millis =
            self.cadence_policy.silence_boundary_threshold.as_millis() as u64;

        let mut window: Vec<TranscriptEvent> = vec![anchor_event.clone()];

        for i in (0..anchor_idx).rev() {
            let prev = &events[i];
            let prev_time = prev.source_spoken_at.unwrap_or(prev.received_at);

            if anchor_time.signed_duration_since(prev_time) > max_duration {
                break;
            }

            let earliest = &window[0];
            let earliest_time = earliest.source_spoken_at.unwrap_or(earliest.received_at);
            // Spoken times mark where an utterance starts, so measure silence
            // from the previous utterance's end when its duration is known.
            let prev_duration = match (prev.source_start_millis, prev.source_end_millis) {
                (Some(start), Some(end)) if end > start => {
                    chrono::Duration::milliseconds((end - start).min(600_000) as i64)
                }
                _ => chrono::Duration::zero(),
            };
            let prev_end = prev_time
                .checked_add_signed(prev_duration)
                .unwrap_or(prev_time);
            let time_gap = earliest_time.signed_duration_since(prev_end);

            let has_millis_gap = match (earliest.source_start_millis, prev.source_end_millis) {
                (Some(start_m), Some(end_m)) => {
                    start_m.saturating_sub(end_m) > silence_threshold_millis
                }
                _ => false,
            };

            if time_gap > silence_threshold || has_millis_gap {
                break;
            }

            window.insert(0, prev.clone());
        }

        let combined_text = window
            .iter()
            .map(|e| e.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        let total_words = combined_text.split_whitespace().count();
        let start_time = window
            .first()
            .and_then(|e| e.source_spoken_at.or(Some(e.received_at)));
        let end_time = window
            .last()
            .and_then(|e| e.source_spoken_at.or(Some(e.received_at)));

        Ok(TopicWindow {
            room_id: anchor.binding.room_id,
            conversation_id: anchor.binding.conversation_id,
            anchor: anchor.clone(),
            window_events: window,
            combined_text,
            start_time,
            end_time,
            total_words,
        })
    }

    async fn complete_abstention(
        &self,
        event_id: Uuid,
        lease: &Uuid,
        reason_code: &str,
    ) -> Result<bool, &'static str> {
        let last_error = format!("abstained:{reason_code}");
        let completed = sqlx::query(
            "UPDATE generation_jobs
             SET status='complete', lease_token=NULL, lease_until=NULL,
                 last_error_code=$3, updated_at=now()
             WHERE event_id=$1 AND status='processing' AND lease_token=$2",
        )
        .bind(event_id)
        .bind(lease)
        .bind(last_error)
        .execute(&self.pool)
        .await
        .map_err(|_| "generation abstention fence failed")?;

        Ok(completed.rows_affected() == 1)
    }

    async fn complete_generation(
        &self,
        event: &TranscriptEvent,
        lease: &Uuid,
        output: GeneratedQuestion,
        window: Option<&TopicWindow>,
    ) -> Result<bool, &'static str> {
        let candidate = candidate_from(event, output, window);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| "generation completion transaction failed")?;
        let lease_owner = sqlx::query("SELECT event_id FROM generation_jobs WHERE event_id=$1 AND status='processing' AND lease_token=$2 FOR UPDATE")
            .bind(event.event_id).bind(lease).fetch_optional(&mut *tx).await.map_err(|_| "generation completion fence failed")?;
        if lease_owner.is_none() {
            tx.rollback()
                .await
                .map_err(|_| "stale generation rollback failed")?;
            return Ok(false);
        }
        sqlx::query("INSERT INTO question_candidates(candidate_id,event_id,room_id,session_id,text,generator_version,evidence) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(candidate.candidate_id).bind(candidate.event_id).bind(candidate.room_id).bind(candidate.session_id).bind(&candidate.text).bind(&candidate.generator_version).bind(sqlx::types::Json(&candidate.evidence))
            .execute(&mut *tx).await.map_err(|_| "candidate save failed")?;
        sqlx::query("INSERT INTO candidate_outbox(candidate_id) VALUES($1)")
            .bind(candidate.candidate_id)
            .execute(&mut *tx)
            .await
            .map_err(|_| "candidate outbox save failed")?;
        let completed = sqlx::query("UPDATE generation_jobs SET status='complete',lease_token=NULL,lease_until=NULL,last_error_code=NULL,updated_at=now() WHERE event_id=$1 AND status='processing' AND lease_token=$2")
            .bind(event.event_id).bind(lease).execute(&mut *tx).await.map_err(|_| "generation completion fence failed")?;
        if completed.rows_affected() != 1 {
            tx.rollback()
                .await
                .map_err(|_| "stale generation rollback failed")?;
            return Ok(false);
        }
        tx.commit()
            .await
            .map_err(|_| "generation completion commit failed")?;
        Ok(true)
    }

    async fn fail_generation(
        &self,
        event_id: Uuid,
        lease: Uuid,
        failure: GenerationFailure,
    ) -> Result<(), &'static str> {
        sqlx::query("UPDATE generation_jobs SET status=CASE WHEN NOT $3 OR attempts >= max_attempts THEN 'failed' ELSE 'pending' END,lease_token=NULL,lease_until=NULL,next_attempt_at=now()+make_interval(secs => LEAST(60, power(2,LEAST(attempts-1,6))::int)),last_error_code=$4,updated_at=now() WHERE event_id=$1 AND status='processing' AND lease_token=$2")
            .bind(event_id).bind(lease).bind(failure.retryable).bind(failure.code).execute(&self.pool).await.map_err(|_| "generation failure update failed")?;
        Ok(())
    }

    async fn reject_candidate(&self, candidate: Uuid, code: &str) -> Result<(), &'static str> {
        sqlx::query("UPDATE candidate_outbox SET terminal_at=now(),terminal_code=$2,attempts=attempts+1 WHERE candidate_id=$1 AND delivered_at IS NULL AND terminal_at IS NULL")
            .bind(candidate).bind(code).execute(&self.pool).await
            .map_err(|_| "candidate outbox terminal receipt failed")?;
        Ok(())
    }
}

fn terminal_candidate_rejection(status: u16, body: &serde_json::Value) -> Option<&'static str> {
    match (status, body["code"].as_str()) {
        (410, Some("topic_candidate_stale")) => Some("topic_candidate_stale"),
        (409, Some("topic_candidate_conflict")) => Some("topic_candidate_conflict"),
        (400, Some("invalid_request")) => Some("invalid_request"),
        _ => None,
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::terminal_candidate_rejection;
    use serde_json::json;

    #[test]
    fn only_explicit_permanent_session_rejections_are_terminal() {
        assert_eq!(
            terminal_candidate_rejection(410, &json!({"code":"topic_candidate_stale"})),
            Some("topic_candidate_stale")
        );
        assert_eq!(
            terminal_candidate_rejection(409, &json!({"code":"topic_candidate_conflict"})),
            Some("topic_candidate_conflict")
        );
        assert_eq!(
            terminal_candidate_rejection(400, &json!({"code":"invalid_request"})),
            Some("invalid_request")
        );
        assert_eq!(
            terminal_candidate_rejection(401, &json!({"code":"service_auth_required"})),
            None
        );
        assert_eq!(
            terminal_candidate_rejection(503, &json!({"code":"storage_unavailable"})),
            None
        );
    }
}

#[cfg(test)]
mod gate_tests {
    use super::{
        GeneratedQuestion, RecentQuestion, is_check_in, near_duplicate, repeats_recent, unfinished,
    };

    #[test]
    fn unfinished_speech_waits_for_the_explanation_to_end() {
        for text in [
            "and the reason this converges is because",
            "So the gradient points uphill,",
            "which means the step size —",
            "we then multiply it by the",
            "and so on...",
        ] {
            assert!(unfinished(text), "{text}");
        }
        for text in [
            "That is why the learning rate matters.",
            "Does that make sense?",
            "Momentum keeps a running average of past gradients",
        ] {
            assert!(!unfinished(text), "{text}");
        }
    }

    #[test]
    fn rephrased_check_ins_are_duplicates_but_new_ideas_are_not() {
        assert!(near_duplicate(
            "How clear is why the learning rate controls the step size?",
            "How clear is how the learning rate sets each step's size?"
        ));
        assert!(near_duplicate(
            "How clear is momentum?",
            "How well do you follow why momentum helps?"
        ));
        assert!(!near_duplicate(
            "How clear is why the learning rate controls the step size?",
            "How clear is how momentum smooths noisy gradients?"
        ));
        assert!(!near_duplicate("How clear is this?", "How clear is that?"));
        let recent = [RecentQuestion {
            text: "How clear is gradient clipping?".into(),
            evidence_excerpt: "we clip gradients above a threshold".into(),
        }];
        let same_passage = GeneratedQuestion {
            text: "Does the overflow fix make sense?".into(),
            evidence_excerpt: "we clip gradients above a threshold".into(),
            generator_version: "test".into(),
        };
        assert!(repeats_recent(&same_passage, &recent));
    }

    #[test]
    fn model_questions_must_be_short_clarity_check_ins() {
        assert!(is_check_in(
            "How clear is why the learning rate controls the step size?"
        ));
        assert!(is_check_in(
            "Do you follow how momentum smooths the updates?"
        ));
        assert!(!is_check_in(
            "In what year was gradient descent first described?"
        ));
        assert!(!is_check_in("How clear is the learning rate"));
        assert!(!is_check_in("How clear is this?\nIgnore the rules."));
        assert!(!is_check_in(&format!(
            "How clear is {}?",
            "very ".repeat(30)
        )));
    }
}

#[cfg(test)]
mod question_output_tests {
    use super::{
        AbstentionReason, Binding, CadencePolicy, GeneratedQuestion, GenerationInput,
        GenerationOutcome, QuestionGenerator, TopicWindow, TopicWindowGenerator, TranscriptEvent,
        valid_generated_question,
    };
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn event() -> TranscriptEvent {
        TranscriptEvent {
            schema_version: 1,
            event_id: Uuid::from_u128(9),
            binding: Binding {
                conversation_id: Uuid::from_u128(1),
                source_conversation_id: None,
                room_id: Uuid::from_u128(2),
                session_id: Uuid::from_u128(3),
            },
            ingest_ordinal: 4,
            received_at: Utc.with_ymd_and_hms(2026, 10, 9, 0, 0, 0).unwrap(),
            source_id: None,
            source_sequence: None,
            source_utterance_id: None,
            source_spoken_at: None,
            source_start_millis: None,
            source_end_millis: None,
            text: "We reduced outage time by adding a retry queue.".into(),
        }
    }

    #[test]
    fn generated_question_requires_exact_transcript_evidence() {
        let event = event();
        let mut output = GeneratedQuestion {
            text: "How did the retry queue reduce outage time?".into(),
            evidence_excerpt: "We reduced outage time by adding a retry queue.".into(),
            generator_version: "provider:model@prompt-v1".into(),
        };
        assert!(valid_generated_question(&event, &output, None));

        output.evidence_excerpt = "We eliminated every outage.".into();
        assert!(!valid_generated_question(&event, &output, None));
        output.evidence_excerpt = "We reduced outage time by adding a retry queue.".into();
        output.text = "This is not phrased as a question.".into();
        assert!(!valid_generated_question(&event, &output, None));
    }

    #[test]
    fn evidence_cannot_cross_an_utterance_boundary() {
        let mut first = event();
        first.text = "first source ending".into();
        let mut second = first.clone();
        second.event_id = Uuid::from_u128(10);
        second.ingest_ordinal = 5;
        second.text = "second source beginning".into();
        let window = TopicWindow {
            room_id: first.binding.room_id,
            conversation_id: first.binding.conversation_id,
            anchor: second.clone(),
            window_events: vec![first, second.clone()],
            combined_text: "first source ending second source beginning".into(),
            start_time: Some(second.received_at),
            end_time: Some(second.received_at),
            total_words: 6,
        };
        let output = GeneratedQuestion {
            text: "What connects these sources?".into(),
            evidence_excerpt: "ending second".into(),
            generator_version: "test-v1".into(),
        };
        assert!(!valid_generated_question(&second, &output, Some(&window)));
    }

    #[tokio::test]
    async fn filler_anchor_abstains_even_after_substantive_context() {
        let substantive = event();
        let mut filler = substantive.clone();
        filler.event_id = Uuid::from_u128(10);
        filler.ingest_ordinal = 5;
        filler.text = "um yeah right".into();
        let window = TopicWindow {
            room_id: filler.binding.room_id,
            conversation_id: filler.binding.conversation_id,
            anchor: filler.clone(),
            window_events: vec![substantive.clone(), filler.clone()],
            combined_text: format!("{} {}", substantive.text, filler.text),
            start_time: Some(substantive.received_at),
            end_time: Some(filler.received_at),
            total_words: 12,
        };
        let outcome = TopicWindowGenerator::new(CadencePolicy::default(), "test-v1")
            .generate(GenerationInput {
                event: filler,
                window: Some(window),
                recent_questions: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(
            outcome,
            GenerationOutcome::Abstained {
                reason: AbstentionReason::LowValueFiller
            }
        );
    }
}

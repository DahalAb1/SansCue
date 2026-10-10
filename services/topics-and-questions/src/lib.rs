use axum::{
    Json, Router,
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

const DEFAULT_GENERATION_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct App {
    pub pool: PgPool,
    pub bee_token: Arc<str>,
}
#[derive(Clone)]
pub struct Delivery {
    pub pool: PgPool,
    pub sessions_url: String,
    pub sessions_token: String,
    client: reqwest::Client,
    generator: Arc<dyn QuestionGenerator>,
    generation_deadline: Duration,
}

pub type GenerationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<GeneratedQuestion, GenerationFailure>> + Send + 'a>>;

/// Provider-neutral generator seam. The current deployment installs only StubV1Generator.
pub trait QuestionGenerator: Send + Sync {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a>;
}

#[derive(Clone, Debug)]
pub struct GenerationInput {
    pub event: TranscriptEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedQuestion {
    pub text: String,
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
        Box::pin(async move { Ok(stub_question(&input.event)) })
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
    let result=sqlx::query("INSERT INTO transcript_events(event_id,schema_version,conversation_id,room_id,session_id,source_conversation_id,ingest_ordinal,received_at,source_id,source_sequence,text,projection_hash) VALUES($1,1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT(event_id) DO NOTHING")
 .bind(event.event_id).bind(event.binding.conversation_id).bind(event.binding.room_id).bind(event.binding.session_id).bind(&event.binding.source_conversation_id).bind(event.ingest_ordinal as i64).bind(event.received_at).bind(&event.source_id).bind(event.source_sequence.map(|s|s.to_string())).bind(&event.text).bind(digest.as_slice()).execute(&mut *tx).await;
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
    {
        Err("invalid event")
    } else {
        Ok(())
    }
}
/// Deliberately transparent deterministic fallback, not a model or production question generator.
pub fn generate(e: &TranscriptEvent) -> Candidate {
    candidate_from(e, stub_question(e))
}

fn stub_question(event: &TranscriptEvent) -> GeneratedQuestion {
    let excerpt = event.text.trim().chars().take(240).collect::<String>();
    let clean = excerpt.trim_end_matches(['.', '?', '!']);
    GeneratedQuestion {
        text: format!("What is the key implication of: {clean}?"),
        generator_version: "stub-v1".into(),
    }
}

fn candidate_from(e: &TranscriptEvent, output: GeneratedQuestion) -> Candidate {
    let excerpt = e.text.trim().chars().take(240).collect::<String>();
    Candidate {
        candidate_id: e.event_id,
        event_id: e.event_id,
        room_id: e.binding.room_id,
        session_id: e.binding.session_id,
        text: output.text,
        generator_version: output.generator_version,
        evidence: Evidence {
            conversation_id: e.binding.conversation_id,
            ingest_ordinal: e.ingest_ordinal,
            received_at: e.received_at,
            excerpt,
        },
        published: false,
    }
}

fn valid_generated_question(output: &GeneratedQuestion) -> bool {
    !output.text.trim().is_empty()
        && output.text.len() <= 1000
        && !output.generator_version.trim().is_empty()
        && output.generator_version.len() <= 128
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
        })
    }

    pub fn with_generator(mut self, generator: Arc<dyn QuestionGenerator>) -> Self {
        self.generator = generator;
        self
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
                if let Err(error) = generator_worker.generate_pending().await {
                    eprintln!("topics-and-questions generation worker failed; retrying ({error})");
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
            let (heartbeat, stop_heartbeat) = self.start_lease_heartbeat(event.event_id, lease);
            let result = tokio::time::timeout(
                self.generation_deadline,
                self.generator.generate(GenerationInput {
                    event: event.clone(),
                }),
            )
            .await;
            // Gracefully stop and join the heartbeat before clearing the lease.
            // This prevents an in-flight extension from racing the retry update.
            let _ = stop_heartbeat.send(true);
            let _ = heartbeat.await;
            match result {
                Ok(Ok(output)) if valid_generated_question(&output) => {
                    if self.complete_generation(&event, &lease, output).await? {
                        completed += 1;
                    }
                }
                Ok(Ok(_)) => {
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
            let event_row = sqlx::query("SELECT schema_version,event_id,conversation_id,room_id,session_id,source_conversation_id,source_id,source_sequence,ingest_ordinal,received_at,text FROM transcript_events WHERE event_id=$1")
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

    async fn complete_generation(
        &self,
        event: &TranscriptEvent,
        lease: &Uuid,
        output: GeneratedQuestion,
    ) -> Result<bool, &'static str> {
        let candidate = candidate_from(event, output);
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

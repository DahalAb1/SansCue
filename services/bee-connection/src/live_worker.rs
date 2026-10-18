use crate::{
    consumer::Consumer,
    domain::{Observation, Signal},
    live::{LiveEventError, MAX_LIVE_EVENT_BYTES, parse_new_utterance},
    pipeline::Disposition,
    store::{Adoption, Store},
};
use chrono::{DateTime, Utc};
use std::{
    ffi::OsString,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::Command,
};
use uuid::Uuid;

const INITIAL_RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);
const HEALTHY_RUN: Duration = Duration::from_secs(30);
const REJECTION_LOG_INTERVAL: Duration = Duration::from_secs(10);

#[allow(async_fn_in_trait)]
pub trait LiveStore: Clone + Send + Sync {
    async fn active_conversations(&self) -> Result<Vec<Uuid>, &'static str>;
    async fn adopt_listening_source(
        &self,
        source_conversation_id: Uuid,
    ) -> Result<Option<Adoption>, &'static str>;
    async fn conversation_for_source(
        &self,
        source_conversation_id: Uuid,
    ) -> Result<Option<Uuid>, &'static str>;
    async fn conversation_for_numeric_source(
        &self,
        numeric_id: u64,
    ) -> Result<Option<Uuid>, &'static str>;
    async fn record_numeric_source_mapping(
        &self,
        numeric_id: u64,
        source_conversation_id: Uuid,
    ) -> Result<(), &'static str>;
    async fn ingest_live(
        &self,
        conversation_id: Uuid,
        observation: crate::domain::Observation,
    ) -> Result<Disposition, &'static str>;
}

impl LiveStore for Store {
    async fn active_conversations(&self) -> Result<Vec<Uuid>, &'static str> {
        Store::active_conversations(self).await
    }
    async fn adopt_listening_source(
        &self,
        source_conversation_id: Uuid,
    ) -> Result<Option<Adoption>, &'static str> {
        Store::adopt_listening_source(self, source_conversation_id).await
    }
    async fn conversation_for_source(
        &self,
        source_conversation_id: Uuid,
    ) -> Result<Option<Uuid>, &'static str> {
        Store::conversation_for_source(self, source_conversation_id).await
    }

    async fn conversation_for_numeric_source(
        &self,
        numeric_id: u64,
    ) -> Result<Option<Uuid>, &'static str> {
        Store::conversation_for_numeric_source(self, numeric_id).await
    }

    async fn record_numeric_source_mapping(
        &self,
        numeric_id: u64,
        source_conversation_id: Uuid,
    ) -> Result<(), &'static str> {
        Store::record_numeric_source_mapping(self, numeric_id, source_conversation_id).await
    }

    async fn ingest_live(
        &self,
        conversation_id: Uuid,
        observation: crate::domain::Observation,
    ) -> Result<Disposition, &'static str> {
        Store::ingest(self, conversation_id, observation).await
    }
}

#[derive(Clone)]
pub struct LiveCliWorker<S = Store> {
    executable: OsString,
    store: S,
    status: Option<Consumer>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveReadSummary {
    pub accepted: usize,
    pub duplicates: usize,
    pub unbound: usize,
    pub rejected: usize,
    pub ignored: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveLineOutcome {
    Accepted,
    Duplicate,
    Unbound,
    Rejected(&'static str),
    Ignored,
}

impl<S: LiveStore> LiveCliWorker<S> {
    pub fn new(executable: impl Into<OsString>, store: S) -> Result<Self, &'static str> {
        let executable = executable.into();
        if executable.is_empty() {
            return Err("Bee CLI path must not be empty");
        }
        Ok(Self {
            executable,
            store,
            status: None,
        })
    }

    /// Reports adopted conversations to sessions so a listening room shows it is live.
    pub fn with_status_reporter(mut self, status: Consumer) -> Self {
        self.status = Some(status);
        self
    }

    /// Resolves the room conversation for a live source, letting a listening room claim it.
    async fn route_source(&self, source: Uuid) -> Result<Option<Uuid>, &'static str> {
        if let Some(conversation) = self.store.conversation_for_source(source).await? {
            return Ok(Some(conversation));
        }
        let Some(adoption) = self.store.adopt_listening_source(source).await? else {
            return Ok(None);
        };
        if let Some(status) = &self.status {
            let summary = serde_json::json!({
                "binding_id": adoption.conversation_id,
                "status": "bound",
                "has_gaps": false,
                "connectivity_revision": adoption.revision,
                "source_conversation_id": source,
            });
            // The adoption is durable; a missed status update only delays the room's label.
            if let Err(error) = status.publish_status(adoption.room_id, &summary).await {
                eprintln!("Bee listening room status update failed ({error})");
            }
        }
        Ok(Some(adoption.conversation_id))
    }

    pub async fn run(self) -> Result<(), &'static str> {
        let mut delay = INITIAL_RESTART_DELAY;
        loop {
            let started = Instant::now();
            if let Err(error) = self.run_once().await {
                eprintln!("bee live stream unavailable; retrying ({error})");
            }
            self.record_transport_signal(Signal::Disconnected).await?;
            let sleep_for = delay;
            if started.elapsed() >= HEALTHY_RUN {
                delay = INITIAL_RESTART_DELAY;
            } else {
                delay = (delay * 2).min(MAX_RESTART_DELAY);
            }
            tokio::time::sleep(sleep_for).await;
        }
    }

    pub async fn record_transport_signal(&self, signal: Signal) -> Result<(), &'static str> {
        for conversation_id in self.store.active_conversations().await? {
            let state = match &signal {
                Signal::Disconnected => "disconnected",
                Signal::Reconnected => "reconnected",
                _ => return Err("invalid live transport signal"),
            };
            let raw = serde_json::to_vec(&serde_json::json!({
                "transport": "bee-cli",
                "state": state,
            }))
            .map_err(|_| "Bee transport signal serialization failed")?;
            self.store
                .ingest_live(
                    conversation_id,
                    Observation {
                        received_at: Utc::now(),
                        raw,
                        decoded: Ok(signal.clone()),
                    },
                )
                .await?;
        }
        Ok(())
    }

    pub async fn consume<R: AsyncRead + Unpin>(
        &self,
        reader: R,
    ) -> Result<LiveReadSummary, &'static str> {
        let mut reader = BufReader::new(reader);
        let mut summary = LiveReadSummary::default();
        let mut last_rejection_log = None;
        while let Some(line) = read_bounded_line(&mut reader)
            .await
            .map_err(|_| "Bee CLI stream read failed")?
        {
            let outcome = match line {
                Ok(line) => self.ingest_line(&line, Utc::now()).await?,
                Err(error) => LiveLineOutcome::Rejected(error.code()),
            };
            match outcome {
                LiveLineOutcome::Accepted => summary.accepted += 1,
                LiveLineOutcome::Duplicate => summary.duplicates += 1,
                LiveLineOutcome::Unbound => summary.unbound += 1,
                LiveLineOutcome::Rejected(code) => {
                    summary.rejected += 1;
                    if last_rejection_log
                        .is_none_or(|logged: Instant| logged.elapsed() >= REJECTION_LOG_INTERVAL)
                    {
                        last_rejection_log = Some(Instant::now());
                        eprintln!("Bee live event rejected ({code})");
                    }
                }
                LiveLineOutcome::Ignored => summary.ignored += 1,
            }
        }
        Ok(summary)
    }

    pub async fn ingest_line(
        &self,
        line: &[u8],
        received_at: DateTime<Utc>,
    ) -> Result<LiveLineOutcome, &'static str> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return Ok(LiveLineOutcome::Ignored);
        }
        // Bee CLI can emit stream timing metadata that is not a product event.
        if let Ok(serde_json::Value::Object(fields)) =
            serde_json::from_slice::<serde_json::Value>(line)
            && fields.len() == 1
            && fields
                .get("timestamp")
                .is_some_and(serde_json::Value::is_number)
        {
            return Ok(LiveLineOutcome::Ignored);
        }
        match parse_new_utterance(line, received_at) {
            Ok(event) => {
                let Some(conversation_id) = self.route_source(event.conversation_uuid).await?
                else {
                    return Ok(LiveLineOutcome::Unbound);
                };
                let observation = event
                    .into_observation()
                    .map_err(|_| "Bee live event normalization failed")?;
                match self.store.ingest_live(conversation_id, observation).await? {
                    Disposition::Accepted { .. } => Ok(LiveLineOutcome::Accepted),
                    Disposition::Duplicate { .. } => Ok(LiveLineOutcome::Duplicate),
                    Disposition::Quarantined { .. } => {
                        Ok(LiveLineOutcome::Rejected("live_event_quarantined"))
                    }
                    Disposition::Control => Ok(LiveLineOutcome::Ignored),
                }
            }
            Err(utterance_error) => match crate::live::parse_lifecycle_event(line, received_at) {
                Ok(lifecycle) => {
                    let conversation_id = match &lifecycle {
                        crate::live::LiveLifecycleEvent::Created(c) => {
                            self.store
                                .record_numeric_source_mapping(c.id, c.uuid)
                                .await?;
                            self.route_source(c.uuid).await?
                        }
                        crate::live::LiveLifecycleEvent::Updated(u) => {
                            self.store.conversation_for_numeric_source(u.id).await?
                        }
                    };
                    let Some(conversation_id) = conversation_id else {
                        return Ok(LiveLineOutcome::Unbound);
                    };
                    let observation = lifecycle
                        .into_observation()
                        .map_err(|_| "Bee live event normalization failed")?;
                    match self.store.ingest_live(conversation_id, observation).await? {
                        Disposition::Accepted { .. } | Disposition::Control => {
                            Ok(LiveLineOutcome::Accepted)
                        }
                        Disposition::Duplicate { .. } => Ok(LiveLineOutcome::Duplicate),
                        Disposition::Quarantined { .. } => {
                            Ok(LiveLineOutcome::Rejected("live_event_quarantined"))
                        }
                    }
                }
                Err(_) => Ok(LiveLineOutcome::Rejected(utterance_error.code())),
            },
        }
    }

    async fn run_once(&self) -> Result<LiveReadSummary, &'static str> {
        let mut child = Command::new(&self.executable)
            .args([
                "stream",
                "--types",
                "new-utterance,new-conversation,update-conversation",
                "--json",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "Bee CLI stream start failed")?;
        let stdout = child.stdout.take().ok_or("Bee CLI stdout unavailable")?;
        let stderr = child.stderr.take().ok_or("Bee CLI stderr unavailable")?;
        self.record_transport_signal(Signal::Reconnected).await?;
        let diagnostics = tokio::spawn(drain_diagnostics(stderr));
        let summary = self.consume(stdout).await;
        if summary.is_err() {
            let _ = child.kill().await;
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "Bee CLI stream wait failed")?;
        let _ = diagnostics.await;
        let summary = summary?;
        if !status.success() {
            return Err("Bee CLI stream exited unsuccessfully");
        }
        Ok(summary)
    }
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<Result<Vec<u8>, LiveEventError>>> {
    let mut line = Vec::new();
    let mut oversized = false;
    let mut saw_bytes = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if !saw_bytes {
                return Ok(None);
            }
            return Ok(Some(if oversized {
                Err(LiveEventError::EventTooLarge)
            } else {
                trim_carriage_return(&mut line);
                Ok(line)
            }));
        }
        saw_bytes = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_length = newline.unwrap_or(available.len());
        if !oversized {
            if line.len() + content_length > MAX_LIVE_EVENT_BYTES {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..content_length]);
            }
        }
        let consumed = content_length + usize::from(newline.is_some());
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(if oversized {
                Err(LiveEventError::EventTooLarge)
            } else {
                trim_carriage_return(&mut line);
                Ok(line)
            }));
        }
    }
}

fn trim_carriage_return(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}

async fn drain_diagnostics(mut stderr: impl AsyncRead + Unpin) {
    let mut buffer = [0_u8; 1024];
    let mut reported = false;
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(_) if !reported => {
                reported = true;
                eprintln!("Bee CLI emitted diagnostic output; content suppressed");
            }
            Ok(_) => {}
        }
    }
}

pub fn configured_cli_path() -> Option<OsString> {
    std::env::var_os("BEE_CLI_PATH").filter(|path| !path.is_empty())
}

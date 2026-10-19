use crate::{
    consumer::Consumer,
    domain::{Observation, Signal},
    live::{LiveEventError, MAX_LIVE_EVENT_BYTES, parse_new_utterance},
    pipeline::Disposition,
    store::Store,
};
use chrono::{DateTime, Utc};
use std::{
    collections::HashMap,
    ffi::OsString,
    process::Stdio,
    sync::{Arc, Mutex},
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
const REPORT_INTERVAL: Duration = Duration::from_secs(5);
/// Unbound recordings stop being offered for selection after this long without speech.
const SIGHTING_RETENTION: chrono::TimeDelta = chrono::TimeDelta::minutes(15);
const MAX_REPORTED_RECORDINGS: usize = 10;

#[allow(async_fn_in_trait)]
pub trait LiveStore: Clone + Send + Sync {
    async fn active_conversations(&self) -> Result<Vec<Uuid>, &'static str>;
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
    live: Arc<Mutex<LiveState>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transport {
    /// The Bee CLI stream is not running; the worker is starting or retrying it.
    #[default]
    Reconnecting,
    /// The Bee CLI stream process is running. This alone does not prove speech delivery.
    Connected,
}

/// A live Bee recording that no room has selected. Only identity, timing and
/// counts are kept; its speech is dropped, never stored or reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sighting {
    pub source_conversation_id: Uuid,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub utterances: u64,
}

#[derive(Default)]
struct LiveState {
    transport: Transport,
    sightings: HashMap<Uuid, Sighting>,
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
            live: Arc::default(),
        })
    }

    /// Reports transport state and unselected recordings to sessions so a
    /// speaker can confirm which recording belongs to their room.
    pub fn with_status_reporter(mut self, status: Consumer) -> Self {
        self.status = Some(status);
        self
    }

    /// Routes a live source only through an explicit sessions binding. A
    /// recording nobody selected is never attached to a room automatically.
    async fn route_source(&self, source: Uuid) -> Result<Option<Uuid>, &'static str> {
        self.store.conversation_for_source(source).await
    }

    fn note_unbound(&self, source: Uuid, at: DateTime<Utc>, utterance: bool) {
        let mut live = self
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sighting = live.sightings.entry(source).or_insert(Sighting {
            source_conversation_id: source,
            first_seen_at: at,
            last_seen_at: at,
            utterances: 0,
        });
        sighting.last_seen_at = sighting.last_seen_at.max(at);
        sighting.utterances += u64::from(utterance);
    }

    fn set_transport(&self, transport: Transport) {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .transport = transport;
    }

    /// The sanitized host-level report sent to sessions: stream state and the
    /// most recently heard unselected recordings, newest first.
    pub fn live_report(&self, now: DateTime<Utc>) -> serde_json::Value {
        live_report(&self.live, now)
    }

    pub async fn run(self) -> Result<(), &'static str> {
        let reporter = self.status.clone().map(|status| {
            let live = self.live.clone();
            tokio::spawn(async move {
                let mut failing = false;
                loop {
                    match status
                        .publish_live_report(&live_report(&live, Utc::now()))
                        .await
                    {
                        Ok(()) => failing = false,
                        Err(error) if !failing => {
                            failing = true;
                            eprintln!("Bee live report delivery failed; retrying ({error})");
                        }
                        Err(_) => {}
                    }
                    tokio::time::sleep(REPORT_INTERVAL).await;
                }
            })
        });
        let result = self.supervise().await;
        if let Some(reporter) = reporter {
            reporter.abort();
        }
        result
    }

    async fn supervise(&self) -> Result<(), &'static str> {
        let mut delay = INITIAL_RESTART_DELAY;
        loop {
            let started = Instant::now();
            if let Err(error) = self.run_once().await {
                eprintln!("bee live stream unavailable; retrying ({error})");
            }
            self.set_transport(Transport::Reconnecting);
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
                    self.note_unbound(event.conversation_uuid, received_at, true);
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
                            let routed = self.route_source(c.uuid).await?;
                            if routed.is_none() {
                                self.note_unbound(c.uuid, received_at, false);
                            }
                            routed
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
        self.set_transport(Transport::Connected);
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

fn live_report(live: &Mutex<LiveState>, now: DateTime<Utc>) -> serde_json::Value {
    let mut live = live.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    live.sightings
        .retain(|_, sighting| now - sighting.last_seen_at <= SIGHTING_RETENTION);
    let mut recordings: Vec<_> = live.sightings.values().cloned().collect();
    recordings.sort_by_key(|sighting| std::cmp::Reverse(sighting.last_seen_at));
    recordings.truncate(MAX_REPORTED_RECORDINGS);
    serde_json::json!({
        "transport": match live.transport {
            Transport::Connected => "connected",
            Transport::Reconnecting => "reconnecting",
        },
        "recordings": recordings.iter().map(|sighting| serde_json::json!({
            "source_conversation_id": sighting.source_conversation_id,
            "first_seen_at": sighting.first_seen_at,
            "last_seen_at": sighting.last_seen_at,
            "utterances": sighting.utterances,
        })).collect::<Vec<_>>(),
    })
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

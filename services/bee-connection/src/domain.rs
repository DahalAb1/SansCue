use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Internal association delivered by sessions, never discovered by querying its DB.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub conversation_id: Uuid,
    /// External source conversation identity, only when supplied by trusted binding setup.
    #[serde(default)]
    pub source_conversation_id: Option<String>,
    pub room_id: Uuid,
    pub session_id: Uuid,
}

/// Canonical SansCue event. Ordinal is arrival order, not speech/source order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationEvent {
    pub schema_version: u16,
    pub event_id: Uuid,
    pub binding: Binding,
    pub ingest_ordinal: u64,
    pub received_at: DateTime<Utc>,
    pub source_id: Option<String>,
    pub source_sequence: Option<u64>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    /// Source conversation identity when the transport actually supplies it.
    #[serde(default)]
    pub source_conversation_id: Option<String>,
    pub source_id: Option<String>,
    /// Only set when the source actually supplies this value.
    pub source_sequence: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Signal {
    Transcript(Transcript),
    Disconnected,
    Reconnected,
    RecoveryUnavailable {
        reason: String,
    },
    /// A transport-independent adapter reports a completed reconciliation.
    /// This is not evidence of a Bee protocol capability by itself.
    RecoveryCompleted {
        through_sequence: Option<u64>,
    },
}

/// Raw bytes survive decoding failures, including non-UTF8 observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub received_at: DateTime<Utc>,
    pub raw: Vec<u8>,
    pub decoded: Result<Signal, String>,
}

/// A source capability assertion, not inferred from an isolated numeric field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub stable_ids: bool,
    pub contiguous_sequence: bool,
}

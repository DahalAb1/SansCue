use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{Observation, Signal, Transcript};

pub const MAX_LIVE_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_UTTERANCE_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveUtterance {
    pub conversation_uuid: Uuid,
    pub utterance_id: u64,
    pub sample_id: Uuid,
    pub start_millis: u64,
    pub end_millis: u64,
    pub text: String,
    pub spoken_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveEventError {
    EventTooLarge,
    InvalidJson,
    UnsupportedEvent,
    InvalidConversation,
    InvalidUtteranceId,
    InvalidSampleId,
    InvalidTimeline,
    InvalidText,
    InvalidSpokenAt,
}

impl LiveEventError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::EventTooLarge => "live_event_too_large",
            Self::InvalidJson => "live_event_invalid_json",
            Self::UnsupportedEvent => "live_event_unsupported",
            Self::InvalidConversation => "live_event_invalid_conversation",
            Self::InvalidUtteranceId => "live_event_invalid_utterance_id",
            Self::InvalidSampleId => "live_event_invalid_sample_id",
            Self::InvalidTimeline => "live_event_invalid_timeline",
            Self::InvalidText => "live_event_invalid_text",
            Self::InvalidSpokenAt => "live_event_invalid_spoken_at",
        }
    }
}

#[derive(Deserialize)]
struct BeeCliEvent {
    #[serde(rename = "type")]
    event_type: String,
    conversation_uuid: String,
    utterance: BeeCliUtterance,
}

#[derive(Deserialize)]
struct BeeCliUtterance {
    id: u64,
    sample_id: String,
    start: f64,
    end: f64,
    text: String,
    spoken_at: String,
}

#[derive(Serialize)]
struct SanitizedEvent<'a> {
    #[serde(rename = "type")]
    event_type: &'static str,
    conversation_uuid: Uuid,
    utterance: SanitizedUtterance<'a>,
}

#[derive(Serialize)]
struct SanitizedUtterance<'a> {
    id: u64,
    sample_id: Uuid,
    start_millis: u64,
    end_millis: u64,
    text: &'a str,
    spoken_at: &'a DateTime<Utc>,
}

impl LiveUtterance {
    /// Serialize only the fields SansCue accepts from the live Bee boundary.
    /// Unknown source fields never enter durable storage or diagnostic output.
    pub fn sanitized_payload(&self) -> Result<Vec<u8>, LiveEventError> {
        serde_json::to_vec(&SanitizedEvent {
            event_type: "new-utterance",
            conversation_uuid: self.conversation_uuid,
            utterance: SanitizedUtterance {
                id: self.utterance_id,
                sample_id: self.sample_id,
                start_millis: self.start_millis,
                end_millis: self.end_millis,
                text: &self.text,
                spoken_at: &self.spoken_at,
            },
        })
        .map_err(|_| LiveEventError::InvalidJson)
    }

    pub fn into_observation(self) -> Result<Observation, LiveEventError> {
        let raw = self.sanitized_payload()?;
        Ok(Observation {
            received_at: self.received_at,
            raw,
            decoded: Ok(Signal::Transcript(Transcript {
                text: self.text,
                source_conversation_id: Some(self.conversation_uuid.to_string()),
                source_id: Some(self.sample_id.to_string()),
                source_sequence: None,
                source_utterance_id: Some(self.utterance_id),
                source_spoken_at: Some(self.spoken_at),
                source_start_millis: Some(self.start_millis),
                source_end_millis: Some(self.end_millis),
            })),
        })
    }
}

/// Parse one JSON line emitted by Bee CLI 0.8.0 for `new-utterance`.
///
/// `received_at` is local ingest time and remains distinct from Bee's
/// source-provided `spoken_at`. Errors expose stable codes, never source data.
pub fn parse_new_utterance(
    bytes: &[u8],
    received_at: DateTime<Utc>,
) -> Result<LiveUtterance, LiveEventError> {
    if bytes.is_empty() || bytes.len() > MAX_LIVE_EVENT_BYTES {
        return Err(LiveEventError::EventTooLarge);
    }
    let event: BeeCliEvent =
        serde_json::from_slice(bytes).map_err(|_| LiveEventError::InvalidJson)?;
    if event.event_type != "new-utterance" {
        return Err(LiveEventError::UnsupportedEvent);
    }
    let conversation_uuid = Uuid::parse_str(&event.conversation_uuid)
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or(LiveEventError::InvalidConversation)?;
    if event.utterance.id == 0 || event.utterance.id > i64::MAX as u64 {
        return Err(LiveEventError::InvalidUtteranceId);
    }
    let sample_id = Uuid::parse_str(&event.utterance.sample_id)
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or(LiveEventError::InvalidSampleId)?;
    let start_millis = seconds_to_millis(event.utterance.start)?;
    let end_millis = seconds_to_millis(event.utterance.end)?;
    if end_millis <= start_millis {
        return Err(LiveEventError::InvalidTimeline);
    }
    if event.utterance.text.trim().is_empty()
        || event.utterance.text.len() > MAX_UTTERANCE_TEXT_BYTES
        || event.utterance.text.contains('\0')
    {
        return Err(LiveEventError::InvalidText);
    }
    let spoken_at = DateTime::parse_from_rfc3339(&event.utterance.spoken_at)
        .map_err(|_| LiveEventError::InvalidSpokenAt)?
        .with_timezone(&Utc);
    Ok(LiveUtterance {
        conversation_uuid,
        utterance_id: event.utterance.id,
        sample_id,
        start_millis,
        end_millis,
        text: event.utterance.text,
        spoken_at,
        received_at,
    })
}

fn seconds_to_millis(seconds: f64) -> Result<u64, LiveEventError> {
    if !seconds.is_finite() || seconds < 0.0 || seconds > i64::MAX as f64 / 1_000.0 {
        return Err(LiveEventError::InvalidTimeline);
    }
    Ok((seconds * 1_000.0).round() as u64)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveConversationCreated {
    pub id: u64,
    pub uuid: Uuid,
    pub state: String,
    pub received_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveConversationUpdated {
    pub id: u64,
    pub state: String,
    pub received_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveLifecycleEvent {
    Created(LiveConversationCreated),
    Updated(LiveConversationUpdated),
}

impl LiveLifecycleEvent {
    pub fn sanitized_payload(&self) -> Result<Vec<u8>, LiveEventError> {
        match self {
            Self::Created(c) => serde_json::to_vec(&serde_json::json!({
                "type": "new-conversation",
                "conversation": {
                    "id": c.id,
                    "uuid": c.uuid,
                    "state": c.state,
                }
            }))
            .map_err(|_| LiveEventError::InvalidJson),
            Self::Updated(u) => serde_json::to_vec(&serde_json::json!({
                "type": "update-conversation",
                "conversation": {
                    "id": u.id,
                    "state": u.state,
                }
            }))
            .map_err(|_| LiveEventError::InvalidJson),
        }
    }

    pub fn into_observation(self) -> Result<Observation, LiveEventError> {
        let raw = self.sanitized_payload()?;
        let (received_at, external_id, state) = match self {
            Self::Created(c) => (c.received_at, Some(c.id), c.state),
            Self::Updated(u) => (u.received_at, Some(u.id), u.state),
        };
        Ok(Observation {
            received_at,
            raw,
            decoded: Ok(Signal::Lifecycle { external_id, state }),
        })
    }
}

#[derive(Deserialize)]
struct BeeCliLifecycleEvent {
    #[serde(rename = "type")]
    event_type: String,
    conversation: Option<BeeCliConversationPayload>,
}

#[derive(Deserialize)]
struct BeeCliConversationPayload {
    id: Option<u64>,
    uuid: Option<String>,
    state: Option<String>,
}

pub fn parse_lifecycle_event(
    bytes: &[u8],
    received_at: DateTime<Utc>,
) -> Result<LiveLifecycleEvent, LiveEventError> {
    if bytes.is_empty() || bytes.len() > MAX_LIVE_EVENT_BYTES {
        return Err(LiveEventError::EventTooLarge);
    }
    let event: BeeCliLifecycleEvent =
        serde_json::from_slice(bytes).map_err(|_| LiveEventError::InvalidJson)?;
    let Some(conv) = event.conversation else {
        return Err(LiveEventError::UnsupportedEvent);
    };
    let id = conv
        .id
        .filter(|&id| id > 0 && id <= i64::MAX as u64)
        .ok_or(LiveEventError::InvalidUtteranceId)?;
    let state = conv.state.unwrap_or_else(|| "processing".to_owned());
    match event.event_type.as_str() {
        "new-conversation" => {
            let uuid_str = conv.uuid.ok_or(LiveEventError::InvalidConversation)?;
            let uuid = Uuid::parse_str(&uuid_str)
                .ok()
                .filter(|u| !u.is_nil())
                .ok_or(LiveEventError::InvalidConversation)?;
            Ok(LiveLifecycleEvent::Created(LiveConversationCreated {
                id,
                uuid,
                state,
                received_at,
            }))
        }
        "update-conversation" => Ok(LiveLifecycleEvent::Updated(LiveConversationUpdated {
            id,
            state,
            received_at,
        })),
        _ => Err(LiveEventError::UnsupportedEvent),
    }
}

use crate::domain::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Disposition {
    Accepted { event_id: Uuid },
    Duplicate { event_id: Uuid },
    Quarantined { reason: String },
    Control,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredObservation {
    pub ordinal: u64,
    pub observation: Observation,
    pub disposition: Disposition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapStatus {
    Suspected,
    Confirmed,
    Recovered,
    Unrecoverable { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub detected_ordinal: u64,
    pub started_at: DateTime<Utc>,
    pub reconnected_at: Option<DateTime<Utc>>,
    /// Inclusive missing sequence interval, only with verified contiguous sequence.
    pub missing: Option<(u64, u64)>,
    pub status: GapStatus,
    pub after_sequence: Option<u64>,
    pub disconnect: bool,
    pub recovered_at_ordinal: Option<u64>,
}

/// One conversation aggregate. Persist atomically before acknowledging ingestion.
/// IDs are assigned only on acceptance; injecting the ID factory makes tests exact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    pub conversation_id: Uuid,
    pub capabilities: Capabilities,
    binding: Option<Binding>,
    observations: Vec<StoredObservation>,
    events: Vec<ConversationEvent>,
    gaps: Vec<Gap>,
    seen_sequences: BTreeSet<u64>,
    high_sequence: Option<u64>,
    active_disconnect: Option<usize>,
}

impl Pipeline {
    pub fn new(conversation_id: Uuid, capabilities: Capabilities) -> Self {
        Self {
            conversation_id,
            capabilities,
            binding: None,
            observations: vec![],
            events: vec![],
            gaps: vec![],
            seen_sequences: BTreeSet::new(),
            high_sequence: None,
            active_disconnect: None,
        }
    }

    pub fn bind(&mut self, binding: Binding) -> Result<(), &'static str> {
        if binding.conversation_id != self.conversation_id
            || binding.room_id.is_nil()
            || binding.session_id.is_nil()
            || binding.conversation_id.is_nil()
        {
            return Err("invalid binding");
        }
        if self
            .binding
            .as_ref()
            .is_some_and(|current| current != &binding)
        {
            return Err("conversation already bound");
        }
        self.binding = Some(binding);
        Ok(())
    }

    pub fn binding(&self) -> Option<&Binding> {
        self.binding.as_ref()
    }
    pub fn events(&self) -> &[ConversationEvent] {
        &self.events
    }
    pub fn observations(&self) -> &[StoredObservation] {
        &self.observations
    }
    pub fn gaps(&self) -> &[Gap] {
        &self.gaps
    }

    pub fn ingest(
        &mut self,
        observation: Observation,
        new_id: impl FnOnce() -> Uuid,
    ) -> Disposition {
        let ordinal = self.observations.len() as u64 + 1;
        let disposition = match &observation.decoded {
            Err(reason) => Disposition::Quarantined {
                reason: format!("decode: {reason}"),
            },
            Ok(Signal::Transcript(transcript)) => {
                self.transcript(transcript, &observation, ordinal, new_id)
            }
            Ok(Signal::Disconnected) => {
                if self.active_disconnect.is_none() {
                    self.active_disconnect = Some(self.gaps.len());
                    self.gaps.push(Gap {
                        detected_ordinal: ordinal,
                        started_at: observation.received_at,
                        reconnected_at: None,
                        missing: None,
                        status: GapStatus::Suspected,
                        after_sequence: self.high_sequence,
                        disconnect: true,
                        recovered_at_ordinal: None,
                    });
                }
                Disposition::Control
            }
            Ok(Signal::Reconnected) => {
                if let Some(index) = self.active_disconnect.take() {
                    self.gaps[index].reconnected_at = Some(observation.received_at);
                }
                // A reconnect alone provides no proof of recovery.
                Disposition::Control
            }
            Ok(Signal::RecoveryUnavailable { reason }) => {
                for gap in &mut self.gaps {
                    if matches!(gap.status, GapStatus::Suspected | GapStatus::Confirmed) {
                        gap.status = GapStatus::Unrecoverable {
                            reason: reason.clone(),
                        };
                    }
                }
                Disposition::Control
            }
        };
        self.observations.push(StoredObservation {
            ordinal,
            observation,
            disposition: disposition.clone(),
        });
        disposition
    }

    fn transcript(
        &mut self,
        transcript: &Transcript,
        observation: &Observation,
        ordinal: u64,
        new_id: impl FnOnce() -> Uuid,
    ) -> Disposition {
        let quarantine = |reason: &str| Disposition::Quarantined {
            reason: reason.into(),
        };
        let Some(binding) = self.binding.clone() else {
            return quarantine("unbound conversation");
        };
        if transcript.text.trim().is_empty()
            || transcript
                .source_id
                .as_ref()
                .is_some_and(|id| id.trim().is_empty())
        {
            return quarantine("empty transcript or source ID");
        }
        let by_id = self.events.iter().find(|event| {
            self.capabilities.stable_ids
                && transcript.source_id.is_some()
                && event.source_id == transcript.source_id
        });
        let by_sequence = self.events.iter().find(|event| {
            self.capabilities.contiguous_sequence
                && transcript.source_sequence.is_some()
                && event.source_sequence == transcript.source_sequence
        });
        if let Some(existing) = by_id.or(by_sequence) {
            if by_id
                .zip(by_sequence)
                .is_some_and(|(a, b)| a.event_id != b.event_id)
                || existing.text != transcript.text
                || existing.source_id != transcript.source_id
                || existing.source_sequence != transcript.source_sequence
            {
                return quarantine("conflicting source identity");
            }
            return Disposition::Duplicate {
                event_id: existing.event_id,
            };
        }
        let event_id = new_id();
        if event_id.is_nil() || self.events.iter().any(|event| event.event_id == event_id) {
            return quarantine("invalid internal event ID");
        }
        if self.capabilities.contiguous_sequence {
            if let Some(sequence) = transcript.source_sequence {
                self.record_sequence(sequence, ordinal, observation.received_at);
            } else {
                // Missing metadata breaks completeness evidence for this observation.
                self.gaps.push(Gap {
                    detected_ordinal: ordinal,
                    started_at: observation.received_at,
                    reconnected_at: None,
                    missing: None,
                    status: GapStatus::Suspected,
                    after_sequence: None,
                    disconnect: false,
                    recovered_at_ordinal: None,
                });
            }
        }
        self.events.push(ConversationEvent {
            schema_version: 1,
            event_id,
            binding,
            ingest_ordinal: ordinal,
            received_at: observation.received_at,
            source_id: transcript.source_id.clone(),
            source_sequence: transcript.source_sequence,
            text: transcript.text.clone(),
        });
        Disposition::Accepted { event_id }
    }

    fn record_sequence(&mut self, sequence: u64, ordinal: u64, at: DateTime<Utc>) {
        if let Some(previous) = self.high_sequence {
            if sequence > previous && sequence - previous > 1 {
                self.gaps.push(Gap {
                    detected_ordinal: ordinal,
                    started_at: at,
                    reconnected_at: None,
                    missing: Some((previous + 1, sequence - 1)),
                    status: GapStatus::Confirmed,
                    after_sequence: Some(previous),
                    disconnect: false,
                    recovered_at_ordinal: None,
                });
            }
        }
        self.seen_sequences.insert(sequence);
        self.high_sequence = Some(
            self.high_sequence
                .map_or(sequence, |high| high.max(sequence)),
        );
        for gap in &mut self.gaps {
            if !matches!(gap.status, GapStatus::Suspected | GapStatus::Confirmed) {
                continue;
            }
            if gap.disconnect && gap.reconnected_at.is_some() && gap.missing.is_none() {
                if let Some(before) = gap.after_sequence {
                    if sequence > before {
                        // First post-reconnect forward observation bounds the interval.
                        if sequence - before == 1 {
                            gap.status = GapStatus::Recovered;
                            gap.recovered_at_ordinal = Some(ordinal);
                        } else {
                            gap.missing = Some((before + 1, sequence - 1));
                            gap.status = GapStatus::Confirmed;
                        }
                    }
                }
            }
            if let Some((start, end)) = gap.missing {
                // Count observed entries instead of expanding potentially huge ranges.
                if self.seen_sequences.range(start..=end).count() as u64 == end - start + 1 {
                    gap.status = GapStatus::Recovered;
                    gap.recovered_at_ordinal = Some(ordinal);
                }
            }
        }
    }
}

use crate::domain::{Capabilities, Observation};
use std::collections::VecDeque;

/// Transport-neutral pull boundary. Production transport/auth/retry is unverified.
/// An adapter must preserve exact input bytes and emit decode errors as observations.
pub trait BeeAdapter {
    fn capabilities(&self) -> Capabilities;
    fn next_observation(&mut self) -> Option<Observation>;
}

/// SansCue test recording format, NOT a claimed Bee wire protocol.
pub struct ReplayAdapter {
    capabilities: Capabilities,
    observations: VecDeque<Observation>,
}

impl ReplayAdapter {
    pub fn new(capabilities: Capabilities, observations: Vec<Observation>) -> Self {
        Self {
            capabilities,
            observations: observations.into(),
        }
    }

    pub fn from_recording(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        #[derive(serde::Deserialize)]
        struct Recording {
            capabilities: Capabilities,
            observations: Vec<Observation>,
        }
        let recording: Recording = serde_json::from_slice(bytes)?;
        Ok(Self::new(recording.capabilities, recording.observations))
    }
}

impl BeeAdapter for ReplayAdapter {
    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
    fn next_observation(&mut self) -> Option<Observation> {
        self.observations.pop_front()
    }
}

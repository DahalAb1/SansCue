use bee_connection::{
    domain::{Observation, Signal},
    live::MAX_LIVE_EVENT_BYTES,
    live_worker::{LiveCliWorker, LiveLineOutcome, LiveReadSummary, LiveStore},
    pipeline::Disposition,
    store::Adoption,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Clone)]
struct MockStore {
    source: Uuid,
    conversation: Uuid,
    seen: Arc<Mutex<HashMap<String, Uuid>>>,
    numeric_mappings: Arc<Mutex<HashMap<u64, Uuid>>>,
    observations: Arc<Mutex<Vec<Observation>>>,
    listening: Arc<Mutex<bool>>,
    adopted: Arc<Mutex<Option<Uuid>>>,
}

impl MockStore {
    fn new(source: Uuid, conversation: Uuid) -> Self {
        Self {
            source,
            conversation,
            seen: Arc::default(),
            numeric_mappings: Arc::default(),
            observations: Arc::default(),
            listening: Arc::default(),
            adopted: Arc::default(),
        }
    }

    fn listening(source: Uuid, conversation: Uuid) -> Self {
        let store = Self::new(source, conversation);
        *store.listening.lock().unwrap() = true;
        store
    }
}

impl LiveStore for MockStore {
    async fn active_conversations(&self) -> Result<Vec<Uuid>, &'static str> {
        Ok(vec![self.conversation])
    }
    async fn conversation_for_source(
        &self,
        source_conversation_id: Uuid,
    ) -> Result<Option<Uuid>, &'static str> {
        let adopted = *self.adopted.lock().unwrap() == Some(source_conversation_id);
        Ok((source_conversation_id == self.source || adopted).then_some(self.conversation))
    }

    async fn adopt_listening_source(
        &self,
        source_conversation_id: Uuid,
    ) -> Result<Option<Adoption>, &'static str> {
        let mut listening = self.listening.lock().unwrap();
        if !*listening {
            return Ok(None);
        }
        *listening = false;
        *self.adopted.lock().unwrap() = Some(source_conversation_id);
        Ok(Some(Adoption {
            room_id: Uuid::new_v4(),
            conversation_id: self.conversation,
            revision: 1,
        }))
    }

    async fn conversation_for_numeric_source(
        &self,
        numeric_id: u64,
    ) -> Result<Option<Uuid>, &'static str> {
        let mappings = self.numeric_mappings.lock().unwrap();
        Ok(mappings
            .get(&numeric_id)
            .and_then(|source_uuid| (*source_uuid == self.source).then_some(self.conversation)))
    }

    async fn record_numeric_source_mapping(
        &self,
        numeric_id: u64,
        source_conversation_id: Uuid,
    ) -> Result<(), &'static str> {
        self.numeric_mappings
            .lock()
            .unwrap()
            .insert(numeric_id, source_conversation_id);
        Ok(())
    }

    async fn ingest_live(
        &self,
        conversation_id: Uuid,
        observation: Observation,
    ) -> Result<Disposition, &'static str> {
        if conversation_id != self.conversation {
            return Err("wrong mock conversation");
        }
        let source_id = match &observation.decoded {
            Ok(Signal::Transcript(transcript)) => transcript
                .source_id
                .clone()
                .ok_or("mock source identity missing")?,
            Ok(Signal::Lifecycle { .. } | Signal::Disconnected | Signal::Reconnected) => {
                self.observations.lock().unwrap().push(observation);
                return Ok(Disposition::Control);
            }
            _ => return Err("mock transcript missing"),
        };
        self.observations.lock().unwrap().push(observation);
        let mut seen = self.seen.lock().unwrap();
        if let Some(event_id) = seen.get(&source_id) {
            return Ok(Disposition::Duplicate {
                event_id: *event_id,
            });
        }
        let event_id = Uuid::new_v4();
        seen.insert(source_id, event_id);
        Ok(Disposition::Accepted { event_id })
    }
}

fn event() -> Value {
    serde_json::from_slice(include_bytes!("../fixtures/bee-cli-new-utterance.json")).unwrap()
}

async fn consume(worker: &LiveCliWorker<MockStore>, input: Vec<u8>) -> LiveReadSummary {
    let (mut writer, reader) = tokio::io::duplex(1024);
    let write = tokio::spawn(async move {
        writer.write_all(&input).await.unwrap();
        writer.shutdown().await.unwrap();
    });
    let summary = worker.consume(reader).await.unwrap();
    write.await.unwrap();
    summary
}

#[tokio::test]
async fn consumes_bounded_lines_and_routes_only_the_bound_conversation() {
    let valid = event();
    let source = Uuid::parse_str(valid["conversation_uuid"].as_str().unwrap()).unwrap();
    let store = MockStore::new(source, Uuid::new_v4());
    let worker = LiveCliWorker::new("bee", store.clone()).unwrap();

    let mut unbound = valid.clone();
    unbound["conversation_uuid"] = json!("33333333-3333-4333-8333-333333333333");
    let mut input = b"  \n{}\n".to_vec();
    input.extend_from_slice(&vec![b'x'; MAX_LIVE_EVENT_BYTES + 1]);
    input.push(b'\n');
    input.extend_from_slice(&serde_json::to_vec(&unbound).unwrap());
    input.push(b'\n');
    input.extend_from_slice(&serde_json::to_vec(&valid).unwrap());
    input.push(b'\n');
    input.extend_from_slice(&serde_json::to_vec(&valid).unwrap());
    input.push(b'\n');

    assert_eq!(
        consume(&worker, input).await,
        LiveReadSummary {
            accepted: 1,
            duplicates: 1,
            unbound: 1,
            rejected: 2,
            ignored: 1,
        }
    );

    let observations = store.observations.lock().unwrap();
    assert_eq!(observations.len(), 2);
    let raw: Value = serde_json::from_slice(&observations[0].raw).unwrap();
    assert!(raw.get("ignored_top_level_field").is_none());
    assert!(raw["utterance"].get("ignored_source_field").is_none());
    let Ok(Signal::Transcript(transcript)) = &observations[0].decoded else {
        panic!("expected normalized transcript");
    };
    assert_eq!(transcript.source_sequence, None);
    assert_eq!(transcript.source_utterance_id, Some(42));
    assert_eq!(transcript.source_start_millis, Some(1_250));
    assert_eq!(transcript.source_end_millis, Some(3_750));
    assert!(transcript.source_spoken_at.is_some());
}

#[tokio::test]
async fn listening_room_adopts_only_the_first_unbound_conversation() {
    let valid = event();
    let first = Uuid::parse_str(valid["conversation_uuid"].as_str().unwrap()).unwrap();
    let store = MockStore::listening(Uuid::new_v4(), Uuid::new_v4());
    let worker = LiveCliWorker::new("bee", store.clone()).unwrap();

    let mut later = valid.clone();
    later["utterance"]["sample_id"] = json!("44444444-4444-4444-8444-444444444444");
    let mut other = valid.clone();
    other["conversation_uuid"] = json!("33333333-3333-4333-8333-333333333333");
    let mut input = Vec::new();
    for line in [&valid, &later, &other] {
        input.extend_from_slice(&serde_json::to_vec(line).unwrap());
        input.push(b'\n');
    }

    assert_eq!(
        consume(&worker, input).await,
        LiveReadSummary {
            accepted: 2,
            duplicates: 0,
            unbound: 1,
            rejected: 0,
            ignored: 0,
        }
    );
    assert_eq!(*store.adopted.lock().unwrap(), Some(first));
}

#[test]
fn rejects_an_empty_cli_path() {
    let store = MockStore::new(Uuid::new_v4(), Uuid::new_v4());
    assert!(LiveCliWorker::new("", store).is_err());
}

#[tokio::test]
async fn transport_restart_records_gap_signals_for_active_conversations() {
    let store = MockStore::new(Uuid::new_v4(), Uuid::new_v4());
    let worker = LiveCliWorker::new("bee", store.clone()).unwrap();

    worker
        .record_transport_signal(Signal::Disconnected)
        .await
        .unwrap();
    worker
        .record_transport_signal(Signal::Reconnected)
        .await
        .unwrap();

    let observations = store.observations.lock().unwrap();
    assert!(matches!(observations[0].decoded, Ok(Signal::Disconnected)));
    assert!(matches!(observations[1].decoded, Ok(Signal::Reconnected)));
}

#[tokio::test]
async fn ignores_timestamps_and_routes_typed_and_structural_lifecycle_events() {
    let source = Uuid::new_v4();
    let store = MockStore::new(source, Uuid::new_v4());
    let worker = LiveCliWorker::new("bee", store.clone()).unwrap();

    assert_eq!(
        worker
            .ingest_line(br#"{"timestamp":1770000000000}"#, chrono::Utc::now())
            .await
            .unwrap(),
        LiveLineOutcome::Ignored
    );

    let created = json!({
        "type": "new-conversation",
        "conversation": {
            "id": 51838,
            "conversation_uuid": source,
            "state": "processing"
        }
    });
    assert_eq!(
        worker
            .ingest_line(&serde_json::to_vec(&created).unwrap(), chrono::Utc::now())
            .await
            .unwrap(),
        LiveLineOutcome::Accepted
    );

    let updated = json!({
        "conversation": {
            "id": 51838,
            "state": "processed"
        }
    });
    assert_eq!(
        worker
            .ingest_line(&serde_json::to_vec(&updated).unwrap(), chrono::Utc::now())
            .await
            .unwrap(),
        LiveLineOutcome::Accepted
    );

    let observations = store.observations.lock().unwrap();
    assert_eq!(observations.len(), 2);
    assert!(matches!(
        observations[0].decoded,
        Ok(Signal::Lifecycle { .. })
    ));
    assert!(matches!(
        observations[1].decoded,
        Ok(Signal::Lifecycle { .. })
    ));
}

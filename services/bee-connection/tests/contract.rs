use bee_connection::{
    adapter::{BeeAdapter, ReplayAdapter},
    domain::*,
};
use uuid::Uuid;

#[test]
fn canonical_event_round_trips_without_invented_source_identity() {
    let event = ConversationEvent {
        schema_version: 1,
        event_id: Uuid::new_v4(),
        binding: Binding {
            conversation_id: Uuid::new_v4(),
            source_conversation_id: None,
            room_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
        },
        ingest_ordinal: 1,
        received_at: "2026-10-08T00:00:00Z".parse().unwrap(),
        source_id: None,
        source_sequence: None,
        text: "Example".into(),
    };
    let bytes = serde_json::to_vec(&event).unwrap();
    assert_eq!(
        serde_json::from_slice::<ConversationEvent>(&bytes).unwrap(),
        event
    );
}

#[test]
fn replay_preserves_malformed_raw_bytes_and_times() {
    let observation = Observation {
        received_at: "2026-10-08T00:00:00Z".parse().unwrap(),
        raw: vec![255, 0],
        decoded: Err("invalid encoding".into()),
    };
    let mut replay = ReplayAdapter::new(Capabilities::default(), vec![observation.clone()]);
    assert_eq!(replay.next_observation(), Some(observation));
    assert_eq!(replay.next_observation(), None);
    assert!(ReplayAdapter::from_recording(b"not json").is_err());
}

#[test]
fn recorded_pipeline_replays_deterministically() {
    use bee_connection::pipeline::{GapStatus, Pipeline};
    let run = || {
        let mut adapter =
            ReplayAdapter::from_recording(include_bytes!("../fixtures/synthetic-recovery.json"))
                .unwrap();
        let mut pipeline = Pipeline::new(Uuid::from_u128(1), adapter.capabilities());
        pipeline
            .bind(Binding {
                conversation_id: Uuid::from_u128(1),
                source_conversation_id: None,
                room_id: Uuid::from_u128(2),
                session_id: Uuid::from_u128(3),
            })
            .unwrap();
        let mut next_id = 100;
        while let Some(observation) = adapter.next_observation() {
            pipeline.ingest(observation, || {
                next_id += 1;
                Uuid::from_u128(next_id)
            });
        }
        pipeline
    };
    let first = run();
    assert_eq!(first, run());
    assert_eq!(first.events().len(), 3);
    assert_eq!(first.observations().len(), 7);
    assert!(
        first
            .gaps()
            .iter()
            .any(|gap| gap.status == GapStatus::Recovered)
    );
    assert!(
        first
            .gaps()
            .iter()
            .any(|gap| gap.status == GapStatus::Suspected),
        "malformed raw observation must remain visible as unresolved evidence"
    );
}

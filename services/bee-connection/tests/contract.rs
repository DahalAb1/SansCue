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

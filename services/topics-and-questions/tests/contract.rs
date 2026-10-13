use chrono::{TimeZone, Utc};
use topics_and_questions::{Binding, TranscriptEvent, generate, validate};
use uuid::Uuid;
fn event() -> TranscriptEvent {
    TranscriptEvent {
        schema_version: 1,
        event_id: Uuid::from_u128(9),
        binding: Binding {
            conversation_id: Uuid::from_u128(1),
            source_conversation_id: None,
            room_id: Uuid::from_u128(2),
            session_id: Uuid::from_u128(3),
        },
        ingest_ordinal: 4,
        received_at: Utc.with_ymd_and_hms(2026, 10, 9, 0, 0, 0).unwrap(),
        source_id: None,
        source_sequence: None,
        text: "The network is unreliable.".into(),
    }
}
#[test]
fn deterministic_stub_is_stable_and_evidence_linked() {
    let e = event();
    let a = generate(&e);
    assert_eq!(a, generate(&e));
    assert_eq!(a.candidate_id, e.event_id);
    assert_eq!(a.event_id, e.event_id);
    assert_eq!(a.generator_version, "stub-v1");
    assert!(!a.published);
    assert_eq!(a.evidence.ingest_ordinal, e.ingest_ordinal);
    assert_eq!(a.evidence.excerpt, e.text);
    assert!(a.text.contains("The network is unreliable?"));
}
#[test]
fn malformed_or_unsupported_transcript_events_are_rejected() {
    let mut e = event();
    assert!(validate(&e).is_ok());
    e.schema_version = 2;
    assert!(validate(&e).is_err());
    e = event();
    e.text = " \n ".into();
    assert!(validate(&e).is_err());
    e = event();
    e.ingest_ordinal = 0;
    assert!(validate(&e).is_err());
    e = event();
    e.binding.room_id = Uuid::nil();
    assert!(validate(&e).is_err());
}

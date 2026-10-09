use bee_connection::{domain::*, pipeline::*};
use uuid::Uuid;

fn binding(id: u128) -> Binding {
    Binding {
        conversation_id: Uuid::from_u128(id),
        source_conversation_id: None,
        room_id: Uuid::from_u128(id + 100),
        session_id: Uuid::from_u128(id + 200),
    }
}
fn pipeline(capabilities: Capabilities) -> Pipeline {
    let mut p = Pipeline::new(binding(1).conversation_id, capabilities);
    p.bind(binding(1)).unwrap();
    p
}
fn observation(signal: Signal) -> Observation {
    Observation {
        received_at: "2026-10-08T00:00:00Z".parse().unwrap(),
        raw: serde_json::to_vec(&signal).unwrap(),
        decoded: Ok(signal),
    }
}
fn transcript(seq: Option<u64>, id: Option<&str>, text: &str) -> Observation {
    observation(Signal::Transcript(Transcript {
        text: text.into(),
        source_conversation_id: None,
        source_id: id.map(str::to_owned),
        source_sequence: seq,
    }))
}

#[test]
fn supplied_source_conversation_must_match_pipeline_identity() {
    let mut p = Pipeline::new(binding(1).conversation_id, evidence());
    let mut bound = binding(1);
    bound.source_conversation_id = Some("source-conv-1".into());
    p.bind(bound).unwrap();
    let mut wrong = transcript(Some(1), Some("a"), "hello");
    if let Ok(Signal::Transcript(value)) = &mut wrong.decoded {
        value.source_conversation_id = Some("source-conv-2".into());
    }
    assert!(matches!(
        ingest(&mut p, wrong),
        Disposition::Quarantined { .. }
    ));
    assert!(p.events().is_empty());
    assert_eq!(p.gaps().len(), 1);
    assert_eq!(p.gaps()[0].status, GapStatus::Suspected);
    let mut absent = transcript(Some(1), Some("b"), "hello");
    if let Ok(Signal::Transcript(value)) = &mut absent.decoded {
        value.source_conversation_id = Some("source-conv-1".into());
    }
    assert!(matches!(
        ingest(&mut p, absent),
        Disposition::Accepted { .. }
    ));
}
fn ingest(p: &mut Pipeline, o: Observation) -> Disposition {
    let id = Uuid::from_u128(p.observations().len() as u128 + 1000);
    p.ingest(o, || id)
}
fn evidence() -> Capabilities {
    Capabilities {
        stable_ids: true,
        contiguous_sequence: true,
    }
}

#[test]
fn binding_is_immutable_idempotent_and_scoped() {
    let mut p = pipeline(evidence());
    p.bind(binding(1)).unwrap();
    assert!(p.bind(binding(2)).is_err());
    let mut changed = binding(1);
    changed.room_id = Uuid::from_u128(900);
    assert!(p.bind(changed).is_err());
    ingest(&mut p, transcript(Some(1), Some("a"), "hello"));
    let mut second = Pipeline::new(binding(2).conversation_id, evidence());
    second.bind(binding(2)).unwrap();
    assert!(matches!(
        ingest(&mut second, transcript(Some(1), Some("a"), "hello")),
        Disposition::Accepted { .. }
    ));
    assert_eq!(p.events()[0].binding, binding(1));
    assert_eq!(second.events()[0].binding, binding(2));
}

#[test]
fn quarantines_unbound_malformed_empty_and_identity_conflicts() {
    let mut p = Pipeline::new(binding(1).conversation_id, evidence());
    assert!(matches!(
        ingest(&mut p, transcript(None, None, "unbound")),
        Disposition::Quarantined { .. }
    ));
    p.bind(binding(1)).unwrap();
    let malformed = Observation {
        received_at: "2026-10-08T00:00:00Z".parse().unwrap(),
        raw: vec![255, 0, 123],
        decoded: Err("malformed".into()),
    };
    ingest(&mut p, malformed.clone());
    ingest(&mut p, transcript(None, None, "  "));
    ingest(&mut p, transcript(Some(1), Some("a"), "original"));
    assert!(matches!(
        ingest(&mut p, transcript(Some(1), Some("a"), "changed")),
        Disposition::Quarantined { .. }
    ));
    assert_eq!(p.observations()[1].observation, malformed);
    assert_eq!(p.events().len(), 1);
    assert_eq!(p.events()[0].ingest_ordinal, 4);
    assert_eq!(
        p.gaps().len(),
        4,
        "decode failure and each rejected transcript remain visible"
    );
    assert_eq!(p.recovery_summary().unresolved_gap_count, 4);
}

#[test]
fn duplicates_reuse_internal_id_and_keep_all_raw_arrivals() {
    let mut p = pipeline(evidence());
    let o = transcript(Some(1), Some("a"), "hello");
    ingest(&mut p, o.clone());
    let id = p.events()[0].event_id;
    assert_eq!(
        p.ingest(o, || panic!("duplicate must not allocate ID")),
        Disposition::Duplicate { event_id: id }
    );
    assert_eq!(p.events().len(), 1);
    assert_eq!(p.observations().len(), 2);
}

#[test]
fn no_source_evidence_never_deduplicates_text_or_claims_recovery() {
    let mut p = pipeline(Capabilities::default());
    ingest(&mut p, transcript(None, None, "same"));
    ingest(&mut p, observation(Signal::Disconnected));
    ingest(&mut p, observation(Signal::Reconnected));
    ingest(&mut p, transcript(None, None, "same"));
    assert_eq!(p.events().len(), 2);
    assert_eq!(p.gaps()[0].status, GapStatus::Suspected);
    assert!(p.gaps()[0].reconnected_at.is_some());
    ingest(
        &mut p,
        observation(Signal::RecoveryUnavailable {
            reason: "no history".into(),
        }),
    );
    assert!(matches!(
        p.gaps()[0].status,
        GapStatus::Unrecoverable { .. }
    ));
}

#[test]
fn disconnect_interval_and_sequence_holes_recover_only_after_all_missing_events() {
    let mut p = pipeline(evidence());
    ingest(&mut p, transcript(Some(10), Some("a"), "ten"));
    ingest(&mut p, observation(Signal::Disconnected));
    ingest(&mut p, observation(Signal::Disconnected));
    assert_eq!(p.gaps().len(), 1);
    ingest(&mut p, observation(Signal::Reconnected));
    ingest(&mut p, transcript(Some(13), Some("d"), "thirteen"));
    assert!(p.gaps().iter().all(|g| g.status == GapStatus::Confirmed));
    ingest(&mut p, transcript(Some(12), Some("c"), "twelve"));
    assert!(p.gaps().iter().all(|g| g.status == GapStatus::Confirmed));
    ingest(&mut p, transcript(Some(11), Some("b"), "eleven"));
    assert!(p.gaps().iter().all(|g| g.status == GapStatus::Recovered));
    assert_eq!(
        p.events()
            .iter()
            .map(|e| e.source_sequence.unwrap())
            .collect::<Vec<_>>(),
        vec![10, 13, 12, 11]
    );
    assert_eq!(
        p.events()
            .iter()
            .map(|e| e.ingest_ordinal)
            .collect::<Vec<_>>(),
        vec![1, 5, 6, 7]
    );
}

#[test]
fn unrecoverable_gap_remains_explicit_even_if_late_data_arrives() {
    let mut p = pipeline(evidence());
    ingest(&mut p, transcript(Some(1), None, "one"));
    ingest(&mut p, transcript(Some(3), None, "three"));
    ingest(
        &mut p,
        observation(Signal::RecoveryUnavailable {
            reason: "history expired".into(),
        }),
    );
    ingest(&mut p, transcript(Some(2), None, "two"));
    assert!(matches!(
        p.gaps()[0].status,
        GapStatus::Unrecoverable { .. }
    ));
}

#[test]
fn successful_reconciliation_updates_summary_without_erasing_failed_attempt() {
    let mut p = pipeline(evidence());
    ingest(&mut p, transcript(Some(1), None, "one"));
    ingest(&mut p, transcript(Some(3), None, "three"));
    ingest(
        &mut p,
        observation(Signal::RecoveryUnavailable {
            reason: "history expired".into(),
        }),
    );
    let historical = p.gaps()[0].clone();
    ingest(&mut p, transcript(Some(2), None, "two"));
    ingest(
        &mut p,
        observation(Signal::RecoveryCompleted {
            through_sequence: Some(3),
        }),
    );
    assert_eq!(p.gaps()[0], historical);
    assert!(matches!(
        p.gaps()[0].status,
        GapStatus::Unrecoverable { .. }
    ));
    assert_eq!(p.recovery_summary().last_completed_ordinal, Some(5));
    assert_eq!(p.recovery_summary().complete_through_sequence, Some(3));
    assert_eq!(p.recovery_summary().unresolved_gap_count, 0);
}

#[test]
fn unverified_numeric_metadata_is_not_sequence_evidence() {
    let mut p = pipeline(Capabilities::default());
    ingest(&mut p, transcript(Some(1), Some("same"), "one"));
    ingest(&mut p, transcript(Some(99), Some("same"), "ninety-nine"));
    assert_eq!(p.events().len(), 2);
    assert!(p.gaps().is_empty());
}

#[test]
fn huge_holes_missing_metadata_and_disconnect_before_first_sequence_are_safe() {
    let mut p = pipeline(evidence());
    ingest(&mut p, observation(Signal::Disconnected));
    ingest(&mut p, observation(Signal::Reconnected));
    ingest(&mut p, transcript(Some(0), None, "zero"));
    ingest(&mut p, transcript(Some(u64::MAX), None, "max"));
    ingest(&mut p, transcript(None, None, "unknown"));
    assert_eq!(p.gaps()[0].status, GapStatus::Suspected);
    assert_eq!(p.gaps()[1].missing, Some((1, u64::MAX - 1)));
    assert_eq!(p.gaps()[2].status, GapStatus::Suspected);
}

#[test]
fn serialization_restart_preserves_deduplication_and_recovery() {
    let mut p = pipeline(evidence());
    ingest(&mut p, transcript(Some(1), Some("a"), "one"));
    ingest(&mut p, observation(Signal::Disconnected));
    let mut restored: Pipeline = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    ingest(&mut restored, observation(Signal::Reconnected));
    assert!(matches!(
        ingest(&mut restored, transcript(Some(1), Some("a"), "one")),
        Disposition::Duplicate { .. }
    ));
    ingest(&mut restored, transcript(Some(2), Some("b"), "two"));
    assert_eq!(restored.gaps()[0].status, GapStatus::Recovered);
}

#[test]
fn internal_uuid_collision_is_quarantined_and_receive_time_is_not_sort_order() {
    let mut p = pipeline(Capabilities::default());
    ingest(&mut p, transcript(None, None, "first"));
    let id = p.events()[0].event_id;
    assert!(matches!(
        p.ingest(transcript(None, None, "second"), || id),
        Disposition::Quarantined { .. }
    ));
    let mut earlier = transcript(None, None, "third");
    earlier.received_at = "2020-01-01T00:00:00Z".parse().unwrap();
    ingest(&mut p, earlier);
    assert_eq!(p.events()[1].ingest_ordinal, 3);
    assert!(p.events()[1].received_at < p.events()[0].received_at);
}

#[test]
fn separate_disconnects_keep_independent_recovery_evidence() {
    let mut p = pipeline(evidence());
    ingest(&mut p, transcript(Some(1), Some("a"), "one"));
    ingest(&mut p, observation(Signal::Disconnected));
    ingest(&mut p, observation(Signal::Reconnected));
    ingest(&mut p, transcript(Some(3), Some("c"), "three"));
    ingest(&mut p, observation(Signal::Disconnected));
    ingest(&mut p, observation(Signal::Reconnected));
    ingest(&mut p, transcript(Some(4), Some("d"), "four"));
    let disconnects: Vec<_> = p.gaps().iter().filter(|g| g.disconnect).collect();
    assert_eq!(disconnects.len(), 2);
    assert_eq!(disconnects[0].status, GapStatus::Confirmed);
    assert_eq!(disconnects[1].status, GapStatus::Recovered);
    ingest(&mut p, transcript(Some(2), Some("b"), "two"));
    assert!(p.gaps().iter().all(|g| g.status == GapStatus::Recovered));
}

#[test]
fn source_id_and_sequence_evidence_cannot_merge_distinct_events() {
    let mut p = pipeline(evidence());
    ingest(&mut p, transcript(Some(1), Some("a"), "same"));
    ingest(&mut p, transcript(Some(2), Some("b"), "same"));
    assert!(matches!(
        ingest(&mut p, transcript(Some(2), Some("a"), "same")),
        Disposition::Quarantined { .. }
    ));
    assert_eq!(p.events().len(), 2);
}

#[test]
fn stable_id_only_deduplicates_without_claiming_sequence_recovery() {
    let mut p = pipeline(Capabilities {
        stable_ids: true,
        contiguous_sequence: false,
    });
    ingest(&mut p, transcript(None, Some("a"), "one"));
    ingest(&mut p, observation(Signal::Disconnected));
    ingest(&mut p, observation(Signal::Reconnected));
    assert!(matches!(
        ingest(&mut p, transcript(None, Some("a"), "one")),
        Disposition::Duplicate { .. }
    ));
    ingest(&mut p, transcript(None, Some("b"), "two"));
    assert_eq!(p.gaps()[0].status, GapStatus::Suspected);
}

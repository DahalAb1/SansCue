use bee_connection::{
    domain::{Binding, Capabilities, Observation, Signal, Transcript},
    pipeline::{Disposition, GapStatus, Pipeline},
};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use uuid::Uuid;

fn sample_binding(conversation_id: Uuid) -> Binding {
    Binding {
        conversation_id,
        source_conversation_id: Some("source-test-conv".into()),
        room_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
    }
}

fn transcript_obs(
    received_at: chrono::DateTime<Utc>,
    spoken_at: chrono::DateTime<Utc>,
    start_millis: u64,
    end_millis: u64,
    source_id: &str,
    text: &str,
) -> Observation {
    Observation {
        received_at,
        raw: vec![1],
        decoded: Ok(Signal::Transcript(Transcript {
            text: text.to_owned(),
            source_conversation_id: Some("source-test-conv".into()),
            source_id: Some(source_id.to_owned()),
            source_sequence: None,
            source_utterance_id: None,
            source_spoken_at: Some(spoken_at),
            source_start_millis: Some(start_millis),
            source_end_millis: Some(end_millis),
        })),
    }
}

#[test]
fn preserves_ingest_order_and_speech_order_separately() {
    let conv_id = Uuid::new_v4();
    let mut pipeline = Pipeline::new(
        conv_id,
        Capabilities {
            stable_ids: true,
            contiguous_sequence: false,
        },
    );
    pipeline.bind(sample_binding(conv_id)).unwrap();

    let t0 = Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 5).unwrap();

    let obs_later_speech = transcript_obs(
        Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 1).unwrap(),
        t1,
        5000,
        7000,
        "sample-second-speech",
        "Second spoken utterance",
    );

    let obs_earlier_speech = transcript_obs(
        Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 2).unwrap(),
        t0,
        0,
        2000,
        "sample-first-speech",
        "First spoken utterance",
    );

    let d1 = pipeline.ingest(obs_later_speech, Uuid::new_v4);
    let d2 = pipeline.ingest(obs_earlier_speech, Uuid::new_v4);

    assert!(matches!(d1, Disposition::Accepted { .. }));
    assert!(matches!(d2, Disposition::Accepted { .. }));

    // Ingest arrival order strictly preserved in events():
    let events = pipeline.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].ingest_ordinal, 1);
    assert_eq!(events[0].text, "Second spoken utterance");
    assert_eq!(events[1].ingest_ordinal, 2);
    assert_eq!(events[1].text, "First spoken utterance");

    // Speech timeline order correctly projected in speech_projection():
    let projected = pipeline.speech_projection();
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].text, "First spoken utterance");
    assert_eq!(projected[1].text, "Second spoken utterance");
    assert_eq!(
        pipeline.speech_projection_text(),
        "First spoken utterance Second spoken utterance"
    );
}

#[test]
fn speech_projection_tolerates_overlap_and_late_arrival() {
    let conv_id = Uuid::new_v4();
    let mut pipeline = Pipeline::new(
        conv_id,
        Capabilities {
            stable_ids: true,
            contiguous_sequence: false,
        },
    );
    pipeline.bind(sample_binding(conv_id)).unwrap();

    let t0 = Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 0).unwrap();

    // Utterance 1: [1000..3000ms]
    let obs1 = transcript_obs(
        Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 1).unwrap(),
        t0,
        1000,
        3000,
        "s1",
        "Hello audience",
    );
    // Utterance 2 (overlapping with Utterance 1 by 500ms): [2500..4500ms]
    let obs2 = transcript_obs(
        Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 2).unwrap(),
        t0,
        2500,
        4500,
        "s2",
        "audience and welcome",
    );
    // Utterance 0 (late packet arriving after 1 & 2): [0..1200ms]
    let obs0 = transcript_obs(
        Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 3).unwrap(),
        t0,
        0,
        1200,
        "s0",
        "Welcome",
    );

    pipeline.ingest(obs1, Uuid::new_v4);
    pipeline.ingest(obs2, Uuid::new_v4);
    pipeline.ingest(obs0, Uuid::new_v4);

    // Ingest order: 1, 2, 0
    assert_eq!(pipeline.events()[0].text, "Hello audience");
    assert_eq!(pipeline.events()[1].text, "audience and welcome");
    assert_eq!(pipeline.events()[2].text, "Welcome");

    // Projected speech order: 0 ([0..1200]), 1 ([1000..3000]), 2 ([2500..4500])
    let projected = pipeline.speech_projection();
    assert_eq!(projected.len(), 3);
    assert_eq!(projected[0].text, "Welcome");
    assert_eq!(projected[1].text, "Hello audience");
    assert_eq!(projected[2].text, "audience and welcome");
}

#[test]
fn reconciliation_after_disconnect_deduplicates_without_duplicate_effects() {
    let conv_id = Uuid::new_v4();
    let mut pipeline = Pipeline::new(
        conv_id,
        Capabilities {
            stable_ids: true,
            contiguous_sequence: false,
        },
    );
    pipeline.bind(sample_binding(conv_id)).unwrap();

    let t0 = Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 0).unwrap();

    let obs1 = transcript_obs(
        Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 1).unwrap(),
        t0,
        0,
        2000,
        "sample-1",
        "Opening passage before disconnect",
    );
    assert!(matches!(
        pipeline.ingest(obs1.clone(), Uuid::new_v4),
        Disposition::Accepted { .. }
    ));

    // Device disconnect occurs
    let disconnect_obs = Observation {
        received_at: Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 2).unwrap(),
        raw: vec![],
        decoded: Ok(Signal::Disconnected),
    };
    pipeline.ingest(disconnect_obs, Uuid::new_v4);

    assert_eq!(pipeline.gaps().len(), 1);
    assert_eq!(pipeline.gaps()[0].status, GapStatus::Suspected);

    // Device reconnects
    let reconnect_obs = Observation {
        received_at: Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 3).unwrap(),
        raw: vec![],
        decoded: Ok(Signal::Reconnected),
    };
    pipeline.ingest(reconnect_obs, Uuid::new_v4);

    // Reconciled replay arrives: obs1 (duplicate) and obs2 (recovered passage)
    let d_dup = pipeline.ingest(obs1, Uuid::new_v4);
    assert!(matches!(d_dup, Disposition::Duplicate { .. }));

    let obs2 = transcript_obs(
        Utc.with_ymd_and_hms(2026, 10, 18, 10, 0, 4).unwrap(),
        t0,
        2500,
        4500,
        "sample-2",
        "Recovered passage after reconnect",
    );
    let d_new = pipeline.ingest(obs2, Uuid::new_v4);
    assert!(matches!(d_new, Disposition::Accepted { .. }));

    // Verify duplicate had no downstream duplicate effect in accepted events or projection
    assert_eq!(pipeline.events().len(), 2);
    assert_eq!(pipeline.speech_projection().len(), 2);
    assert_eq!(
        pipeline.speech_projection_text(),
        "Opening passage before disconnect Recovered passage after reconnect"
    );
}

#[test]
fn conversation_lifecycle_tracks_external_uuid_and_numeric_id() {
    let conv_id = Uuid::new_v4();
    let mut pipeline = Pipeline::new(
        conv_id,
        Capabilities {
            stable_ids: true,
            contiguous_sequence: false,
        },
    );
    pipeline.bind(sample_binding(conv_id)).unwrap();

    assert_eq!(pipeline.external_id(), None);
    assert_eq!(pipeline.lifecycle_state(), None);

    let created_obs = Observation {
        received_at: Utc::now(),
        raw: vec![],
        decoded: Ok(Signal::Lifecycle {
            external_id: Some(51838),
            state: "processing".into(),
        }),
    };
    let d1 = pipeline.ingest(created_obs, Uuid::new_v4);
    assert_eq!(d1, Disposition::Control);
    assert_eq!(pipeline.external_id(), Some(51838));
    assert_eq!(pipeline.lifecycle_state(), Some("processing"));

    let updated_obs = Observation {
        received_at: Utc::now(),
        raw: vec![],
        decoded: Ok(Signal::Lifecycle {
            external_id: Some(51838),
            state: "processed".into(),
        }),
    };
    let d2 = pipeline.ingest(updated_obs, Uuid::new_v4);
    assert_eq!(d2, Disposition::Control);
    assert_eq!(pipeline.external_id(), Some(51838));
    assert_eq!(pipeline.lifecycle_state(), Some("processed"));
}

#[test]
fn fixture_converges_to_ordered_projection() {
    let fixture_bytes = include_bytes!("../fixtures/speech-ordering-recovery.json");
    let fixture: Value = serde_json::from_slice(fixture_bytes).unwrap();

    let conv_id = Uuid::new_v4();
    let mut pipeline = Pipeline::new(
        conv_id,
        Capabilities {
            stable_ids: fixture["capabilities"]["stable_ids"].as_bool().unwrap(),
            contiguous_sequence: fixture["capabilities"]["contiguous_sequence"]
                .as_bool()
                .unwrap(),
        },
    );
    pipeline.bind(sample_binding(conv_id)).unwrap();

    let observations: Vec<Observation> =
        serde_json::from_value(fixture["observations"].clone()).unwrap();
    let mut accepted_count = 0;
    let mut duplicate_count = 0;

    for obs in observations {
        match pipeline.ingest(obs, Uuid::new_v4) {
            Disposition::Accepted { .. } => accepted_count += 1,
            Disposition::Duplicate { .. } => duplicate_count += 1,
            _ => {}
        }
    }

    assert_eq!(accepted_count, 4);
    assert_eq!(duplicate_count, 1);
    assert_eq!(pipeline.events().len(), 4);

    let projection = pipeline.speech_projection();
    assert_eq!(projection.len(), 4);

    assert_eq!(
        pipeline.speech_projection_text(),
        "First in speech, but arrived second. Second in speech, but arrived first. Third in speech, overlapping with fourth. Overlapping speech segment."
    );
}

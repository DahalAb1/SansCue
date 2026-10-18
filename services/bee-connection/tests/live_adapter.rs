use bee_connection::live::{
    LiveEventError, MAX_LIVE_EVENT_BYTES, MAX_UTTERANCE_TEXT_BYTES, parse_new_utterance,
};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

fn received_at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 18, 1, 0, 5).unwrap()
}

fn valid_event() -> Value {
    serde_json::from_slice(include_bytes!("../fixtures/bee-cli-new-utterance.json")).unwrap()
}

#[test]
fn parses_observed_bee_cli_shape_and_separates_source_time() {
    let event = parse_new_utterance(
        include_bytes!("../fixtures/bee-cli-new-utterance.json"),
        received_at(),
    )
    .unwrap();

    assert_eq!(
        event.conversation_uuid,
        Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()
    );
    assert_eq!(event.utterance_id, Some(42));
    assert_eq!(
        event.sample_id,
        Some(Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap())
    );
    assert_eq!(event.start_millis, Some(1_250));
    assert_eq!(event.end_millis, Some(3_750));
    assert_eq!(event.text, "A sanitized live transcript example.");
    assert_eq!(
        event.spoken_at,
        Some(Utc.with_ymd_and_hms(2026, 10, 18, 1, 0, 0).unwrap())
    );
    assert_eq!(event.received_at, received_at());
}

#[test]
fn parses_current_documented_structural_payload_without_inventing_identity() {
    let payload = json!({
        "conversation_uuid": "11111111-1111-4111-8111-111111111111",
        "utterance": {
            "text": "A documented minimal Bee utterance.",
            "speaker": "speaker_1"
        }
    });
    let event = parse_new_utterance(&serde_json::to_vec(&payload).unwrap(), received_at()).unwrap();
    assert_eq!(event.utterance_id, None);
    assert_eq!(event.sample_id, None);
    assert_eq!(event.start_millis, None);
    assert_eq!(event.end_millis, None);
    assert_eq!(event.spoken_at, None);
    let observation = event.into_observation().unwrap();
    let bee_connection::domain::Signal::Transcript(transcript) = observation.decoded.unwrap()
    else {
        panic!("expected transcript");
    };
    assert_eq!(transcript.source_id, None);
    assert_eq!(transcript.source_sequence, None);
}

#[test]
fn sanitized_payload_discards_every_unknown_field() {
    let event = parse_new_utterance(
        include_bytes!("../fixtures/bee-cli-new-utterance.json"),
        received_at(),
    )
    .unwrap();
    let stored: Value = serde_json::from_slice(&event.sanitized_payload().unwrap()).unwrap();

    assert_eq!(stored["type"], "new-utterance");
    assert_eq!(stored["utterance"]["start_millis"], 1_250);
    assert_eq!(stored["utterance"]["end_millis"], 3_750);
    assert!(stored.get("ignored_top_level_field").is_none());
    assert!(stored["utterance"].get("ignored_source_field").is_none());
    assert!(stored.get("location").is_none());
}

#[test]
fn preserves_source_order_metadata_independently_of_arrival() {
    let mut later = valid_event();
    later["utterance"]["sample_id"] = json!("33333333-3333-4333-8333-333333333333");
    later["utterance"]["start"] = json!(9.0);
    later["utterance"]["end"] = json!(10.0);
    later["utterance"]["spoken_at"] = json!("2026-10-18T01:00:09Z");
    let later = parse_new_utterance(&serde_json::to_vec(&later).unwrap(), received_at()).unwrap();

    let earlier = parse_new_utterance(
        include_bytes!("../fixtures/bee-cli-new-utterance.json"),
        received_at() + chrono::Duration::seconds(1),
    )
    .unwrap();

    assert!(earlier.received_at > later.received_at);
    assert!(earlier.spoken_at < later.spoken_at);
    assert!(earlier.start_millis < later.start_millis);
}

#[test]
fn repeated_source_event_keeps_the_same_identity() {
    let bytes = include_bytes!("../fixtures/bee-cli-new-utterance.json");
    let first = parse_new_utterance(bytes, received_at()).unwrap();
    let repeated =
        parse_new_utterance(bytes, received_at() + chrono::Duration::seconds(1)).unwrap();

    assert_eq!(first.conversation_uuid, repeated.conversation_uuid);
    assert_eq!(first.sample_id, repeated.sample_id);
    assert_ne!(first.received_at, repeated.received_at);
}

#[test]
fn rejects_unsupported_malformed_and_unbounded_events_with_stable_codes() {
    let cases = [
        (json!({}), LiveEventError::InvalidJson),
        (
            {
                let mut value = valid_event();
                value["type"] = json!("update-conversation");
                value
            },
            LiveEventError::UnsupportedEvent,
        ),
        (
            {
                let mut value = valid_event();
                value["conversation_uuid"] = json!(Uuid::nil());
                value
            },
            LiveEventError::InvalidConversation,
        ),
        (
            {
                let mut value = valid_event();
                value["utterance"]["id"] = json!(0);
                value
            },
            LiveEventError::InvalidUtteranceId,
        ),
        (
            {
                let mut value = valid_event();
                value["utterance"]["sample_id"] = json!("not-a-uuid");
                value
            },
            LiveEventError::InvalidSampleId,
        ),
        (
            {
                let mut value = valid_event();
                value["utterance"]["end"] = json!(0.5);
                value
            },
            LiveEventError::InvalidTimeline,
        ),
        (
            {
                let mut value = valid_event();
                value["utterance"]["text"] = json!("   ");
                value
            },
            LiveEventError::InvalidText,
        ),
        (
            {
                let mut value = valid_event();
                value["utterance"]["spoken_at"] = json!("not-a-time");
                value
            },
            LiveEventError::InvalidSpokenAt,
        ),
    ];

    for (value, expected) in cases {
        let error =
            parse_new_utterance(&serde_json::to_vec(&value).unwrap(), received_at()).unwrap_err();
        assert_eq!(error, expected);
        assert!(error.code().starts_with("live_event_"));
    }

    let oversized = vec![b'x'; MAX_LIVE_EVENT_BYTES + 1];
    assert_eq!(
        parse_new_utterance(&oversized, received_at()).unwrap_err(),
        LiveEventError::EventTooLarge
    );

    let mut long_text = valid_event();
    long_text["utterance"]["text"] = json!("x".repeat(MAX_UTTERANCE_TEXT_BYTES + 1));
    assert_eq!(
        parse_new_utterance(&serde_json::to_vec(&long_text).unwrap(), received_at()).unwrap_err(),
        LiveEventError::InvalidText
    );
}

#[test]
fn parses_conversation_lifecycle_events_and_sanitizes_payload() {
    use bee_connection::live::{
        LiveConversationCreated, LiveConversationUpdated, LiveLifecycleEvent, parse_lifecycle_event,
    };

    let new_conv = json!({
        "type": "new-conversation",
        "conversation": {
            "id": 123,
            "uuid": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "state": "processing",
            "title": "Private user title to discard"
        }
    });

    let parsed =
        parse_lifecycle_event(&serde_json::to_vec(&new_conv).unwrap(), received_at()).unwrap();
    match parsed {
        LiveLifecycleEvent::Created(LiveConversationCreated {
            id, uuid, state, ..
        }) => {
            assert_eq!(id, 123);
            assert_eq!(
                uuid,
                Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap()
            );
            assert_eq!(state, "processing");
        }
        _ => panic!("expected Created lifecycle event"),
    }

    let update_conv = json!({
        "type": "update-conversation",
        "conversation": {
            "id": 123,
            "state": "processed",
            "title": "Another private title",
            "short_summary": "Private summary"
        }
    });

    let parsed_update =
        parse_lifecycle_event(&serde_json::to_vec(&update_conv).unwrap(), received_at()).unwrap();
    match parsed_update {
        LiveLifecycleEvent::Updated(LiveConversationUpdated { id, state, .. }) => {
            assert_eq!(id, 123);
            assert_eq!(state, "processed");
        }
        _ => panic!("expected Updated lifecycle event"),
    }
}

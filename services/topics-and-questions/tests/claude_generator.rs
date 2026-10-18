use axum::{Json, Router, http::HeaderMap, routing::post};
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use topics_and_questions::{
    AbstentionReason, Binding, GenerationInput, GenerationOutcome, QuestionGenerator, TopicWindow,
    TopicWindowGenerator, TranscriptEvent, claude::ClaudeGenerator,
};
use uuid::Uuid;

const SPEECH: &str = "The learning rate decides how far each gradient step moves the weights.";

/// Serves one canned Messages API reply and records the request it received.
async fn mock_api(status: u16, reply: Value) -> (String, Arc<Mutex<Option<(HeaderMap, Value)>>>) {
    let seen = Arc::new(Mutex::new(None));
    let record = seen.clone();
    let app = Router::new().route(
        "/v1/messages",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let record = record.clone();
            let reply = reply.clone();
            async move {
                *record.lock().unwrap() = Some((headers, body));
                (
                    axum::http::StatusCode::from_u16(status).unwrap(),
                    Json(reply),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), seen)
}

fn input(text: &str) -> GenerationInput {
    let event = TranscriptEvent {
        schema_version: 1,
        event_id: Uuid::new_v4(),
        binding: Binding {
            conversation_id: Uuid::new_v4(),
            source_conversation_id: None,
            room_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
        },
        ingest_ordinal: 1,
        received_at: Utc::now(),
        source_id: None,
        source_sequence: None,
        source_utterance_id: None,
        source_spoken_at: None,
        source_start_millis: None,
        source_end_millis: None,
        text: text.to_owned(),
    };
    let window = TopicWindow {
        room_id: event.binding.room_id,
        conversation_id: event.binding.conversation_id,
        anchor: event.clone(),
        window_events: vec![event.clone()],
        combined_text: text.to_owned(),
        start_time: None,
        end_time: None,
        total_words: text.split_whitespace().count(),
    };
    GenerationInput {
        event,
        window: Some(window),
    }
}

fn generator(base: &str) -> ClaudeGenerator {
    ClaudeGenerator::new(
        TopicWindowGenerator::new(Default::default(), "gate"),
        "test-key",
        Some(base),
        None,
    )
    .unwrap()
}

fn answer(abstain: bool, question: &str, quote: &str) -> Value {
    let text = json!({"abstain": abstain, "question": question, "evidence_quote": quote});
    json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": text.to_string()}]})
}

#[tokio::test]
async fn grounded_model_question_becomes_a_candidate() {
    let (base, seen) = mock_api(
        200,
        answer(
            false,
            "How clear is what the learning rate controls?",
            "how far each gradient step moves",
        ),
    )
    .await;
    let outcome = generator(&base).generate(input(SPEECH)).await.unwrap();
    let GenerationOutcome::Candidate(question) = outcome else {
        panic!("expected a candidate, got {outcome:?}");
    };
    assert_eq!(
        question.text,
        "How clear is what the learning rate controls?"
    );
    assert_eq!(
        question.evidence_excerpt,
        "how far each gradient step moves"
    );
    assert_eq!(question.generator_version, "claude-opus-5-5/topic-check-v1");

    let (headers, body) = seen.lock().unwrap().clone().unwrap();
    assert_eq!(headers["x-api-key"], "test-key");
    assert_eq!(headers["anthropic-version"], "2023-06-01");
    assert_eq!(body["model"], "claude-opus-5-5");
    assert!(
        body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains(SPEECH)
    );
}

#[tokio::test]
async fn model_abstention_and_ungrounded_quotes_publish_nothing() {
    let (base, _) = mock_api(200, answer(true, "", "")).await;
    assert_eq!(
        generator(&base).generate(input(SPEECH)).await.unwrap(),
        GenerationOutcome::Abstained {
            reason: AbstentionReason::ModelAbstained
        }
    );
    let (base, _) = mock_api(
        200,
        answer(false, "How clear is momentum?", "momentum smooths updates"),
    )
    .await;
    assert_eq!(
        generator(&base).generate(input(SPEECH)).await.unwrap(),
        GenerationOutcome::Abstained {
            reason: AbstentionReason::WeaklyGrounded
        }
    );
}

#[tokio::test]
async fn filler_skips_the_model_and_overload_is_retryable() {
    let (base, seen) = mock_api(529, json!({"type": "error"})).await;
    assert_eq!(
        generator(&base)
            .generate(input("um okay so"))
            .await
            .unwrap(),
        GenerationOutcome::Abstained {
            reason: AbstentionReason::LowValueFiller
        }
    );
    assert!(
        seen.lock().unwrap().is_none(),
        "filler must not cost a model call"
    );
    let failure = generator(&base).generate(input(SPEECH)).await.unwrap_err();
    assert_eq!(failure.code, "model_unavailable");
    assert!(failure.retryable);
}

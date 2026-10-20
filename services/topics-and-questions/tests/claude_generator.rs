use axum::{Json, Router, http::HeaderMap, routing::post};
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use topics_and_questions::{
    AbstentionReason, Binding, GenerationInput, GenerationOutcome, QuestionGenerator,
    RecentQuestion, TopicWindow, TopicWindowGenerator, TranscriptEvent,
    claude::{BedrockAuth, ClaudeGenerator},
};
use uuid::Uuid;

const SPEECH: &str = "The learning rate decides how far each gradient step moves the weights.";

#[tokio::test]
#[ignore = "requires a private AWS credentials file; makes three small billable Nova requests"]
async fn nova_live_grounded_candidates() {
    let file = std::env::var("NOVA_TEST_CREDENTIALS_FILE").expect("credentials file required");
    let generator = ClaudeGenerator::bedrock(
        TopicWindowGenerator::new(Default::default(), "gate"),
        BedrockAuth::CredentialsFile(file.into()),
        "us-east-1",
        Some("amazon.nova-micro-v1:0"),
        None,
    )
    .unwrap();
    for speech in [
        SPEECH,
        "Photosynthesis converts light energy into chemical energy. Plants use sunlight, water, and carbon dioxide to make glucose and release oxygen.",
        "A hash table uses a hash function to choose a bucket for each key. Collisions happen when different keys choose the same bucket. Chaining stores those entries together in a list.",
    ] {
        let outcome = generator.generate(input(speech)).await.unwrap();
        let GenerationOutcome::Candidate(question) = outcome else {
            panic!("expected grounded candidate, got {outcome:?}");
        };
        assert_eq!(
            question.generator_version,
            "amazon.nova-micro-v1:0/topic-check-v1"
        );
        assert!(speech.contains(&question.evidence_excerpt));
        println!("PASS Nova grounded question: {}", question.text);
    }
}

/// Explicit paid smoke check only; ordinary test runs never use live credentials.
#[tokio::test]
#[ignore = "requires private Cloudflare token/account files and makes one billable request"]
async fn cloudflare_live_grounded_candidate() {
    let token_file = std::env::var("CLOUDFLARE_TEST_TOKEN_FILE").expect("token file path required");
    let account_file =
        std::env::var("CLOUDFLARE_TEST_ACCOUNT_FILE").expect("account file path required");
    let token = std::fs::read_to_string(token_file).expect("read private token file");
    let token = token
        .trim()
        .strip_prefix("CLOUDFLARE_API_TOKEN=")
        .unwrap_or(token.trim())
        .trim_matches(['\'', '"']);
    let account = std::fs::read_to_string(account_file).expect("read account ID file");
    let generator = ClaudeGenerator::cloudflare(
        TopicWindowGenerator::new(Default::default(), "gate"),
        token,
        account.trim(),
        None,
    )
    .unwrap();
    let outcome = generator.generate(input(SPEECH)).await.unwrap();
    let GenerationOutcome::Candidate(question) = outcome else {
        panic!("expected a grounded candidate, got {outcome:?}");
    };
    assert_eq!(
        question.generator_version,
        "anthropic/claude-haiku-5.5/topic-check-v1"
    );
    println!(
        "Live Cloudflare Haiku returned a grounded candidate: {}",
        question.text
    );
}

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
        recent_questions: Vec::new(),
    }
}

fn generator(base: &str) -> ClaudeGenerator {
    ClaudeGenerator::anthropic(
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
async fn factual_quiz_output_is_rejected_and_recent_drafts_reach_the_prompt() {
    let (base, seen) = mock_api(
        200,
        answer(
            false,
            "What does the learning rate decide?",
            "how far each gradient step moves",
        ),
    )
    .await;
    let mut request = input(SPEECH);
    request.recent_questions = vec![RecentQuestion {
        text: "How clear is gradient descent?".into(),
        evidence_excerpt: "gradient".into(),
    }];
    assert_eq!(
        generator(&base).generate(request).await.unwrap(),
        GenerationOutcome::Abstained {
            reason: AbstentionReason::UnsuitableQuestion
        }
    );
    let (_, body) = seen.lock().unwrap().clone().unwrap();
    let content = body["messages"][0]["content"].as_str().unwrap();
    assert!(
        content.contains(
            "<earlier_check_ins>\n- How clear is gradient descent?\n</earlier_check_ins>"
        )
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

#[tokio::test]
async fn bedrock_backend_signs_invoke_model_with_file_credentials() {
    let reply = answer(
        false,
        "How clear is what the learning rate controls?",
        "how far each gradient step moves",
    );
    let seen = Arc::new(Mutex::new(None));
    let record = seen.clone();
    let app = Router::new().route(
        "/model/{model}/invoke",
        post(
            move |axum::extract::Path(model): axum::extract::Path<String>,
                  headers: HeaderMap,
                  Json(body): Json<Value>| {
                let record = record.clone();
                let reply = reply.clone();
                async move {
                    *record.lock().unwrap() = Some((model, headers, body));
                    Json(reply)
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let credentials = std::env::temp_dir().join(format!("sanscue-bedrock-{}.json", Uuid::new_v4()));
    let generator = ClaudeGenerator::bedrock(
        TopicWindowGenerator::new(Default::default(), "gate"),
        BedrockAuth::CredentialsFile(credentials.clone()),
        "us-east-1",
        None,
        Some(&base),
    )
    .unwrap();
    // Until the host helper writes credentials, generation retries instead of failing.
    let missing = generator.generate(input(SPEECH)).await.unwrap_err();
    assert_eq!(missing.code, "model_credentials_unavailable");
    assert!(missing.retryable);

    let expired = json!({"Version": 1, "AccessKeyId": "AKIDEXAMPLE", "SecretAccessKey": "secret", "SessionToken": "token", "Expiration": "2000-01-01T00:00:00Z"});
    std::fs::write(&credentials, expired.to_string()).unwrap();
    assert!(
        generator
            .generate(input(SPEECH))
            .await
            .unwrap_err()
            .retryable
    );

    let fresh = json!({"Version": 1, "AccessKeyId": "AKIDEXAMPLE", "SecretAccessKey": "secret", "SessionToken": "token", "Expiration": (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339()});
    std::fs::write(&credentials, fresh.to_string()).unwrap();
    let GenerationOutcome::Candidate(question) = generator.generate(input(SPEECH)).await.unwrap()
    else {
        panic!("expected a candidate");
    };
    std::fs::remove_file(&credentials).unwrap();
    assert_eq!(
        question.generator_version,
        "us.anthropic.claude-haiku-4-5-20251001-v1:0/topic-check-v1"
    );
    let (model, headers, body) = seen.lock().unwrap().clone().unwrap();
    assert_eq!(model, "us.anthropic.claude-haiku-4-5-20251001-v1:0");
    let authorization = headers["authorization"].to_str().unwrap();
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"));
    assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
    assert_eq!(headers["x-amz-security-token"], "token");
    assert_eq!(body["anthropic_version"], "bedrock-2023-05-31");
}

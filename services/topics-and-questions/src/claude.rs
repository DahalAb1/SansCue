//! Claude-backed question generator. Deterministic topic-window checks run
//! first, so silence, filler and cooldown never cost a model call.
use crate::{
    AbstentionReason, GeneratedQuestion, GenerationFailure, GenerationFuture, GenerationInput,
    GenerationOutcome, QuestionGenerator, TopicWindow, TopicWindowGenerator,
};
use chrono::Utc;
use serde_json::{Value, json};
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Speech older than this is skipped so a slow model never questions stale content.
const MAX_ANCHOR_AGE: Duration = Duration::from_secs(45);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);

const SYSTEM_PROMPT: &str = "You help a speaker check, during a live talk or lecture, whether the audience is following. \
You receive the most recent stretch of the speaker's speech as numbered transcript lines. The transcript comes from speech \
recognition and can contain recognition errors.\n\n\
Decide whether the speech contains one concept the speaker explained that is worth a quick comprehension check right now. \
If it does, write one check-in question. Each listener answers it privately with Clear, Partly clear, or Need help, so the \
question asks how clear the concept is rather than testing a fact. Example: \"How clear is why the learning rate controls \
the step size?\"\n\n\
Question rules: at most 20 words; one concept that the transcript itself explains; plain words an audience member would \
use; ends with a question mark; no answer options, names, or line numbers.\n\n\
Evidence rules: evidence_quote must be copied character for character from a single transcript line, without the line \
number. Choose the 5 to 30 words that best show where the concept was explained.\n\n\
Set abstain to true, with empty question and evidence_quote, when the speech is a greeting, logistics, small talk, a \
question to the room, too fragmentary to understand, or contains no teachable idea. Abstaining is better than a vague \
question.";

#[derive(Clone)]
pub struct ClaudeGenerator {
    gate: TopicWindowGenerator,
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
    version: String,
    /// Server-side refusal fallbacks exist only on Anthropic's first-party API.
    fallbacks: bool,
}

impl ClaudeGenerator {
    pub fn new(
        gate: TopicWindowGenerator,
        api_key: &str,
        base_url: Option<&str>,
        model: Option<&str>,
    ) -> Result<Self, &'static str> {
        let base = base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/');
        let url = reqwest::Url::parse(base).map_err(|_| "invalid Anthropic base URL")?;
        if url.scheme() != "https" && url.host_str() != Some("127.0.0.1") {
            return Err("Anthropic base URL must use HTTPS");
        }
        let model = model.unwrap_or(DEFAULT_MODEL).trim();
        if api_key.trim().is_empty() || model.is_empty() || model.len() > 100 {
            return Err("invalid Anthropic model configuration");
        }
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| "HTTP client initialization failed")?;
        Ok(Self {
            gate,
            client,
            endpoint: format!("{base}/v1/messages"),
            api_key: api_key.trim().to_owned(),
            version: format!("{model}/topic-check-v1"),
            model: model.to_owned(),
            fallbacks: base == DEFAULT_BASE_URL,
        })
    }

    fn request(&self, window: &TopicWindow) -> Value {
        let lines = window
            .window_events
            .iter()
            .map(|event| event.text.trim())
            .filter(|text| !text.is_empty())
            .enumerate()
            .map(|(index, text)| format!("[{}] {text}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let mut request = json!({
            "model": self.model,
            "max_tokens": 4000,
            "system": SYSTEM_PROMPT,
            "messages": [{
                "role": "user",
                "content": format!("<transcript>\n{lines}\n</transcript>"),
            }],
            "output_config": {
                // Low effort keeps the check-in timely while the speaker is still on the topic.
                "effort": "low",
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "abstain": {"type": "boolean"},
                            "question": {"type": "string"},
                            "evidence_quote": {"type": "string"},
                        },
                        "required": ["abstain", "question", "evidence_quote"],
                        "additionalProperties": false,
                    },
                },
            },
        });
        if self.fallbacks {
            request["fallbacks"] = json!("default");
        }
        request
    }

    async fn ask(&self, window: &TopicWindow) -> Result<GenerationOutcome, GenerationFailure> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01");
        if self.fallbacks {
            request = request.header("anthropic-beta", "server-side-fallback-2026-07-01");
        }
        let response = request
            .json(&self.request(window))
            .send()
            .await
            .map_err(|_| failure("model_unavailable", true))?;
        let status = response.status();
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(failure("model_unavailable", true));
        }
        if !status.is_success() {
            return Err(failure("model_request_rejected", false));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| failure("model_response_invalid", true))?;
        let answer = parse_answer(&body)?;
        if answer.abstain {
            return Ok(GenerationOutcome::Abstained {
                reason: AbstentionReason::ModelAbstained,
            });
        }
        let Some(evidence_excerpt) = ground_quote(window, &answer.evidence_quote) else {
            return Ok(GenerationOutcome::Abstained {
                reason: AbstentionReason::WeaklyGrounded,
            });
        };
        Ok(GenerationOutcome::Candidate(GeneratedQuestion {
            text: answer.question.trim().to_owned(),
            evidence_excerpt,
            generator_version: self.version.clone(),
        }))
    }
}

impl QuestionGenerator for ClaudeGenerator {
    fn generate<'a>(&'a self, input: GenerationInput) -> GenerationFuture<'a> {
        Box::pin(async move {
            let age = Utc::now() - input.event.received_at;
            if age.to_std().is_ok_and(|age| age > MAX_ANCHOR_AGE) {
                return Ok(GenerationOutcome::Abstained {
                    reason: AbstentionReason::StaleTranscript,
                });
            }
            let window = input.window.clone();
            if let GenerationOutcome::Abstained { reason } = self.gate.generate(input).await? {
                return Ok(GenerationOutcome::Abstained { reason });
            }
            let window = window.ok_or_else(|| failure("topic_window_unavailable", true))?;
            self.ask(&window).await
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Answer {
    abstain: bool,
    question: String,
    evidence_quote: String,
}

fn failure(code: &'static str, retryable: bool) -> GenerationFailure {
    GenerationFailure { code, retryable }
}

fn parse_answer(body: &Value) -> Result<Answer, GenerationFailure> {
    match body["stop_reason"].as_str() {
        Some("refusal") => return Err(failure("model_refused", false)),
        Some("max_tokens") => return Err(failure("model_output_truncated", false)),
        _ => {}
    }
    let text = body["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<String>();
    let answer: Value =
        serde_json::from_str(&text).map_err(|_| failure("model_response_invalid", false))?;
    let field = |name: &str| answer[name].as_str().map(str::to_owned);
    match (
        answer["abstain"].as_bool(),
        field("question"),
        field("evidence_quote"),
    ) {
        (Some(abstain), Some(question), Some(evidence_quote)) => Ok(Answer {
            abstain,
            question,
            evidence_quote,
        }),
        _ => Err(failure("model_response_invalid", false)),
    }
}

/// Returns the transcript's own spelling of the model's quote, so evidence is
/// always an exact substring of one source event.
fn ground_quote(window: &TopicWindow, quote: &str) -> Option<String> {
    let quote = quote.trim();
    if quote.is_empty() {
        return None;
    }
    let lowered = quote.to_lowercase();
    window.window_events.iter().find_map(|event| {
        if event.text.contains(quote) {
            return Some(quote.to_owned());
        }
        let text = event.text.to_lowercase();
        // Only reuse byte offsets when lowercasing kept the text's length.
        if text.len() != event.text.len() || lowered.len() != quote.len() {
            return None;
        }
        let start = text.find(&lowered)?;
        event
            .text
            .get(start..start + quote.len())
            .map(str::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Binding, TranscriptEvent};
    use uuid::Uuid;

    fn window(lines: &[&str]) -> TopicWindow {
        let binding = Binding {
            conversation_id: Uuid::new_v4(),
            source_conversation_id: None,
            room_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
        };
        let events = lines
            .iter()
            .enumerate()
            .map(|(index, text)| TranscriptEvent {
                schema_version: 1,
                event_id: Uuid::new_v4(),
                binding: binding.clone(),
                ingest_ordinal: index as u64 + 1,
                received_at: Utc::now(),
                source_id: None,
                source_sequence: None,
                source_utterance_id: None,
                source_spoken_at: None,
                source_start_millis: None,
                source_end_millis: None,
                text: (*text).to_owned(),
            })
            .collect::<Vec<_>>();
        TopicWindow {
            room_id: binding.room_id,
            conversation_id: binding.conversation_id,
            anchor: events.last().unwrap().clone(),
            window_events: events,
            combined_text: lines.join(" "),
            start_time: None,
            end_time: None,
            total_words: 0,
        }
    }

    #[test]
    fn quotes_are_grounded_in_the_transcript_spelling() {
        let window = window(&["So the Learning Rate sets how far each step moves."]);
        assert_eq!(
            ground_quote(&window, "the learning rate sets how far"),
            Some("the Learning Rate sets how far".into())
        );
        assert_eq!(ground_quote(&window, "momentum keeps velocity"), None);
        assert_eq!(ground_quote(&window, "  "), None);
    }

    #[test]
    fn structured_answers_parse_and_refusals_fail_closed() {
        let body = json!({
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "text", "text": "{\"abstain\":false,\"question\":\"How clear is the step size?\",\"evidence_quote\":\"each step\"}"},
            ],
        });
        assert_eq!(
            parse_answer(&body).unwrap(),
            Answer {
                abstain: false,
                question: "How clear is the step size?".into(),
                evidence_quote: "each step".into(),
            }
        );
        let refused = json!({"stop_reason": "refusal", "content": []});
        assert_eq!(parse_answer(&refused).unwrap_err().code, "model_refused");
        let malformed =
            json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "{}"}]});
        assert!(!parse_answer(&malformed).unwrap_err().retryable);
    }

    #[test]
    fn request_uses_structured_output_and_numbered_lines() {
        let generator = ClaudeGenerator::new(
            TopicWindowGenerator::new(Default::default(), "gate"),
            "test-key",
            None,
            None,
        )
        .unwrap();
        let request = generator.request(&window(&["first idea", "second idea"]));
        assert_eq!(request["model"], DEFAULT_MODEL);
        assert_eq!(request["output_config"]["format"]["type"], "json_schema");
        assert_eq!(
            request["messages"][0]["content"],
            "<transcript>\n[1] first idea\n[2] second idea\n</transcript>"
        );
        assert!(
            ClaudeGenerator::new(
                TopicWindowGenerator::new(Default::default(), "gate"),
                "key",
                Some("http://example.com"),
                None,
            )
            .is_err()
        );
    }
}

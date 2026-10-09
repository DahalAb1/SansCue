use crate::store::Store;
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone)]
pub struct TopicsConsumer {
    client: reqwest::Client,
    url: String,
    token: String,
    store: Store,
}
impl TopicsConsumer {
    pub fn new(url: &str, token: &str, store: Store) -> Result<Self, &'static str> {
        let parsed = reqwest::Url::parse(url).map_err(|_| "invalid topics internal URL")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || token.is_empty()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err("invalid topics consumer configuration");
        };
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|_| "HTTP client initialization failed")?,
            url: url.trim_end_matches('/').into(),
            token: token.into(),
            store,
        })
    }
    pub async fn poll_once(&self) -> Result<usize, &'static str> {
        let rows = self.store.pending_topics().await?;
        let mut done = 0;
        let mut retryable_failure = false;
        for (id, payload) in rows {
            let result = self
                .client
                .post(format!("{}/internal/v1/transcript-events", self.url))
                .bearer_auth(&self.token)
                .json(&payload)
                .send()
                .await;
            let result = match result {
                Ok(response) => response,
                Err(_) => {
                    retryable_failure = true;
                    continue;
                }
            };
            if !result.status().is_success() {
                let status = result.status().as_u16();
                let body: Value = result.json().await.unwrap_or(Value::Null);
                if let Some(code) = terminal_rejection(status, &body) {
                    self.store.reject_topic(id, code).await?;
                    done += 1;
                } else {
                    retryable_failure = true;
                }
                continue;
            };
            let body: Value = match result.json().await {
                Ok(body) => body,
                Err(_) => {
                    retryable_failure = true;
                    continue;
                }
            };
            if body["accepted"] != true
                || body["event_id"]
                    .as_str()
                    .and_then(|value| Uuid::parse_str(value).ok())
                    != Some(id)
            {
                retryable_failure = true;
                continue;
            };
            if self.store.acknowledge_topic(id).await.is_err() {
                retryable_failure = true;
                continue;
            }
            done += 1;
        }
        if retryable_failure {
            Err("some Topics event deliveries remain pending for retry")
        } else {
            Ok(done)
        }
    }
    pub async fn run(self) -> Result<(), &'static str> {
        loop {
            if let Err(error) = self.poll_once().await {
                eprintln!("bee-connection topics delivery failed; retrying ({error})");
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

fn terminal_rejection(status: u16, body: &Value) -> Option<&'static str> {
    match (status, body["code"].as_str()) {
        (400, Some("invalid_transcript_event")) => Some("invalid_transcript_event"),
        (409, Some("event_identity_conflict")) => Some("event_identity_conflict"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::terminal_rejection;
    use serde_json::json;

    #[test]
    fn only_explicit_permanent_topics_rejections_are_terminal() {
        assert_eq!(
            terminal_rejection(400, &json!({"code":"invalid_transcript_event"})),
            Some("invalid_transcript_event")
        );
        assert_eq!(
            terminal_rejection(409, &json!({"code":"event_identity_conflict"})),
            Some("event_identity_conflict")
        );
        assert_eq!(
            terminal_rejection(401, &json!({"code":"service_auth_required"})),
            None
        );
        assert_eq!(
            terminal_rejection(503, &json!({"code":"storage_unavailable"})),
            None
        );
        assert_eq!(
            terminal_rejection(400, &json!({"code":"some_future_error"})),
            None
        );
    }
}

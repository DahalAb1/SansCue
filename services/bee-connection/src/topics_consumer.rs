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
        for (id, payload) in rows {
            let result = self
                .client
                .post(format!("{}/internal/v1/transcript-events", self.url))
                .bearer_auth(&self.token)
                .json(&payload)
                .send()
                .await
                .map_err(|_| "topics event delivery failed")?;
            if !result.status().is_success() {
                return Err("topics event delivery rejected");
            };
            let body: Value = result
                .json()
                .await
                .map_err(|_| "topics event acknowledgement invalid")?;
            if body["accepted"] != true
                || body["event_id"]
                    .as_str()
                    .and_then(|value| Uuid::parse_str(value).ok())
                    != Some(id)
            {
                return Err("topics event acknowledgement invalid");
            };
            self.store.acknowledge_topic(id).await?;
            done += 1;
        }
        Ok(done)
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

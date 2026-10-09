//! Device-independent sessions command consumer. It handles only canonical
//! SansCue bind/unbind commands; transport-specific adapters remain separate.
use crate::{
    domain::Binding,
    store::{CommandOutcome, Store},
};
use serde_json::{Value, json};
use uuid::Uuid;

#[allow(async_fn_in_trait)]
pub trait CommandStore: Clone + Send + Sync {
    async fn ensure_command_conversation(&self, conversation: Uuid) -> Result<(), &'static str>;
    async fn apply_command(
        &self,
        command: Uuid,
        revision: i64,
        action: &str,
        binding: Binding,
    ) -> Result<CommandOutcome, &'static str>;
}

impl CommandStore for Store {
    async fn ensure_command_conversation(&self, conversation: Uuid) -> Result<(), &'static str> {
        Store::ensure_command_conversation(self, conversation).await
    }
    async fn apply_command(
        &self,
        command: Uuid,
        revision: i64,
        action: &str,
        binding: Binding,
    ) -> Result<CommandOutcome, &'static str> {
        Store::apply_command(self, command, revision, action, binding).await
    }
}

#[derive(Clone)]
pub struct Consumer<S = Store> {
    client: reqwest::Client,
    base: String,
    token: String,
    store: S,
}

impl Consumer<Store> {
    pub fn new(base: &str, token: &str, store: Store) -> Result<Self, &'static str> {
        Self::with_store(base, token, store)
    }
}

impl<S: CommandStore> Consumer<S> {
    pub fn with_store(base: &str, token: &str, store: S) -> Result<Self, &'static str> {
        let url = reqwest::Url::parse(base).map_err(|_| "invalid sessions internal URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || token.is_empty()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("invalid sessions consumer configuration");
        }
        let base = base.trim_end_matches('/').to_owned();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| "HTTP client initialization failed")?;
        Ok(Self {
            client,
            base,
            token: token.to_owned(),
            store,
        })
    }

    pub async fn poll_once(&self) -> Result<usize, &'static str> {
        let response = self
            .client
            .get(format!("{}/internal/v1/bee/commands", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "sessions command poll failed")?;
        if !response.status().is_success() {
            return Err("sessions command poll rejected");
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| "sessions command payload invalid")?;
        let commands = body["commands"]
            .as_array()
            .ok_or("sessions command list invalid")?;
        let mut completed = 0;
        for command in commands {
            let (id, revision, action, binding) = parse_command(command)?;
            self.store
                .ensure_command_conversation(binding.conversation_id)
                .await?;
            let outcome = self
                .store
                .apply_command(id, revision, action, binding.clone())
                .await?;
            if !should_publish_status(outcome) {
                self.acknowledge(id).await?;
                completed += 1;
                continue;
            }
            let summary = if action == "bind" {
                json!({"binding_id":binding.conversation_id,"status":"bound","has_gaps":false,"connectivity_revision":revision})
            } else {
                json!({"binding_id":null,"status":"unbound","has_gaps":false,"connectivity_revision":revision})
            };
            let status = self
                .client
                .post(format!(
                    "{}/internal/v1/bee/rooms/{}/status",
                    self.base, binding.room_id
                ))
                .bearer_auth(&self.token)
                .json(&summary)
                .send()
                .await
                .map_err(|_| "sessions status delivery failed")?;
            if !status.status().is_success() {
                let code = status.status().as_u16();
                let body = status.json::<Value>().await.unwrap_or(Value::Null);
                if is_superseded_status(code, &body) {
                    self.acknowledge(id).await?;
                    completed += 1;
                    continue;
                }
                return Err("sessions status delivery rejected");
            }
            self.acknowledge(id).await?;
            completed += 1;
        }
        Ok(completed)
    }

    async fn acknowledge(&self, command: Uuid) -> Result<(), &'static str> {
        let response = self
            .client
            .post(format!(
                "{}/internal/v1/bee/commands/{command}/ack",
                self.base
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| "sessions command acknowledgement failed")?;
        if !response.status().is_success() {
            return Err("sessions command acknowledgement rejected");
        }
        Ok(())
    }

    pub async fn run(self) -> Result<(), &'static str> {
        loop {
            if let Err(error) = self.poll_once().await {
                // Keep retrying transient service failures; don't log tokens or payloads.
                eprintln!("bee-connection worker delivery failed; retrying ({error})");
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
}

fn should_publish_status(outcome: CommandOutcome) -> bool {
    outcome == CommandOutcome::Applied
}

fn is_superseded_status(http_status: u16, body: &Value) -> bool {
    http_status == 409 && body["code"] == "bee_binding_stale"
}

fn parse_command(command: &Value) -> Result<(Uuid, i64, &'static str, Binding), &'static str> {
    let id = Uuid::parse_str(command["command_id"].as_str().ok_or("command ID missing")?)
        .map_err(|_| "command ID invalid")?;
    let action = match command["type"].as_str().ok_or("command type missing")? {
        "bind" => "bind",
        "unbind" => "unbind",
        _ => return Err("unsupported command type"),
    };
    let revision = command["revision"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or("command revision invalid")?;
    let payload = &command["payload"];
    let parse = |key: &str| {
        Uuid::parse_str(payload[key].as_str().ok_or("binding identity missing")?)
            .map_err(|_| "binding identity invalid")
    };
    let binding = Binding {
        conversation_id: parse("conversation_id")?,
        room_id: parse("room_id")?,
        session_id: parse("session_id")?,
        source_conversation_id: payload["source_conversation_id"]
            .as_str()
            .map(str::to_owned),
    };
    Ok((id, revision, action, binding))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_payload_requires_authorized_session_identity() {
        let id = Uuid::new_v4();
        let command = json!({"command_id":Uuid::new_v4(),"revision":1,"type":"bind","payload":{"conversation_id":id,"room_id":Uuid::new_v4(),"session_id":Uuid::new_v4()}});
        let (_, _, action, binding) = parse_command(&command).unwrap();
        assert_eq!(action, "bind");
        assert!(!binding.session_id.is_nil());
        let invalid = json!({"command_id":Uuid::new_v4(),"revision":1,"type":"bind","payload":{"conversation_id":id,"room_id":Uuid::new_v4()}});
        assert!(parse_command(&invalid).is_err());
    }

    #[test]
    fn stale_commands_are_acknowledged_without_status_publication() {
        assert!(!should_publish_status(CommandOutcome::Stale));
        assert!(should_publish_status(CommandOutcome::Applied));
    }

    #[test]
    fn only_explicit_stale_status_conflict_is_retired() {
        assert!(is_superseded_status(
            409,
            &json!({"code":"bee_binding_stale"})
        ));
        assert!(!is_superseded_status(
            409,
            &json!({"code":"bee_binding_mismatch"})
        ));
        assert!(!is_superseded_status(
            401,
            &json!({"code":"bee_binding_stale"})
        ));
        assert!(!is_superseded_status(
            500,
            &json!({"code":"bee_binding_stale"})
        ));
    }
}

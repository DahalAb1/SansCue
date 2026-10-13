use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use bee_connection::{
    consumer::{CommandStore, Consumer},
    domain::Binding,
    store::CommandOutcome,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone, Default)]
struct MockStore {
    latest_revision: Arc<Mutex<i64>>,
    applied: Arc<Mutex<Vec<i64>>>,
}

impl CommandStore for MockStore {
    async fn ensure_command_conversation(&self, _: Uuid) -> Result<(), &'static str> {
        Ok(())
    }
    async fn apply_command(
        &self,
        _: Uuid,
        revision: i64,
        _: &str,
        _: Binding,
    ) -> Result<CommandOutcome, &'static str> {
        let mut latest = self.latest_revision.lock().unwrap();
        if revision <= *latest {
            return Ok(CommandOutcome::Stale);
        }
        *latest = revision;
        self.applied.lock().unwrap().push(revision);
        Ok(CommandOutcome::Applied)
    }
}

#[derive(Clone, Default)]
struct MockSessions {
    commands: Arc<Vec<Value>>,
    status_revisions: Arc<Mutex<Vec<i64>>>,
    acknowledgements: Arc<Mutex<Vec<String>>>,
}

async fn commands(State(state): State<MockSessions>) -> Json<Value> {
    Json(json!({"commands":state.commands.as_ref()}))
}

async fn status(
    State(state): State<MockSessions>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let revision = body["connectivity_revision"].as_i64().unwrap_or_default();
    state.status_revisions.lock().unwrap().push(revision);
    if revision == 1 {
        (
            StatusCode::CONFLICT,
            Json(json!({"code":"bee_binding_stale"})),
        )
    } else {
        (StatusCode::OK, Json(json!({"accepted":true})))
    }
}

async fn acknowledge(State(state): State<MockSessions>, Path(id): Path<String>) -> Json<Value> {
    state.acknowledgements.lock().unwrap().push(id);
    Json(json!({"accepted":true}))
}

fn command(
    id: Uuid,
    revision: i64,
    kind: &str,
    room: Uuid,
    conversation: Uuid,
    session: Uuid,
) -> Value {
    json!({"command_id":id,"revision":revision,"type":kind,"payload":{"room_id":room,"conversation_id":conversation,"session_id":session}})
}

#[tokio::test]
async fn superseded_status_is_retired_and_next_command_progresses() {
    let room = Uuid::new_v4();
    let conversation = Uuid::new_v4();
    let session = Uuid::new_v4();
    let bind_id = Uuid::new_v4();
    let unbind_id = Uuid::new_v4();
    // Sessions has already advanced to r2 when the worker polls; r1's status
    // is obsolete even though the mocked Bee store can apply it first.
    let state = MockSessions {
        commands: Arc::new(vec![
            command(bind_id, 1, "bind", room, conversation, session),
            command(unbind_id, 2, "unbind", room, conversation, session),
        ]),
        ..Default::default()
    };
    let app = Router::new()
        .route("/internal/v1/bee/commands", get(commands))
        .route("/internal/v1/bee/rooms/{room}/status", post(status))
        .route("/internal/v1/bee/commands/{id}/ack", post(acknowledge))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let store = MockStore::default();
    let worker = Consumer::with_store(
        &format!("http://{address}"),
        "synthetic-token",
        store.clone(),
    )
    .unwrap();
    assert_eq!(worker.poll_once().await.unwrap(), 2);
    assert_eq!(*store.applied.lock().unwrap(), vec![1, 2]);
    assert_eq!(*state.status_revisions.lock().unwrap(), vec![1, 2]);
    assert_eq!(
        *state.acknowledgements.lock().unwrap(),
        vec![bind_id.to_string(), unbind_id.to_string()]
    );
    server.abort();
}

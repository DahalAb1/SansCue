//! Authenticated, room-scoped delivery for committed room invalidations.
//! Durable events are read from PostgreSQL so reconnects and process restarts
//! cannot silently lose a committed change.

use crate::{access::Access, security::hash};
use axum::{
    Router,
    extract::{OriginalUri, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use cookie::Cookie;
use futures_util::StreamExt;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Duration, interval, timeout},
};
use uuid::Uuid;

const POLL_INTERVAL: Duration = Duration::from_millis(400);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_EVENTS_PER_POLL: i64 = 50;
const MAX_CONNECTIONS: usize = 64;
static CONNECTIONS: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub fn router(access: Access) -> Router {
    Router::new()
        .route("/rooms/{room}/events", get(connect))
        .with_state(access)
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    Cookie::split_parse(headers.get(header::COOKIE)?.to_str().ok()?)
        .filter_map(Result::ok)
        .find(|cookie| cookie.name() == "sanscue_session")
        .map(|cookie| cookie.value().to_owned())
}

fn reject(status: StatusCode) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")]).into_response()
}

async fn connect(
    State(access): State<Access>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    // Browser WebSockets do not provide a CSRF header. Requiring the exact
    // configured Origin, a same-origin session cookie and active membership
    // provides the same cross-site protection as the JSON read endpoints.
    let Some(expected_origin) = access.security.origin.as_deref() else {
        return reject(StatusCode::SERVICE_UNAVAILABLE);
    };
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(expected_origin) {
        return reject(StatusCode::FORBIDDEN);
    }
    let Some(presented) = cookie_token(&headers) else {
        return reject(StatusCode::UNAUTHORIZED);
    };
    let path = uri.path();
    let Some(room_part) = path
        .strip_prefix("/rooms/")
        .and_then(|v| v.strip_suffix("/events"))
    else {
        return reject(StatusCode::NOT_FOUND);
    };
    let Ok(room_id) = Uuid::parse_str(room_part) else {
        return reject(StatusCode::NOT_FOUND);
    };
    let after = match parse_after(uri.query()) {
        Some(value) => value,
        None => return reject(StatusCode::BAD_REQUEST),
    };
    let digest = hash(presented.as_bytes());
    let row = match sqlx::query(
        "SELECT r.sequence,r.status AS room_status,m.role,m.status AS member_status,EXISTS(SELECT 1 FROM browser_sessions op WHERE op.token_hash=$2 AND op.operator_epoch=$3 AND op.created_at>now()-interval '24 hours' AND op.last_seen_at>now()-interval '12 hours') AS is_operator FROM rooms r JOIN memberships m ON m.room_id=r.id LEFT JOIN browser_sessions s ON s.id=m.session_id WHERE r.id=$1 AND ((s.token_hash=$2 AND s.created_at>now()-interval '24 hours' AND s.last_seen_at>now()-interval '12 hours') OR (m.role='speaker' AND EXISTS(SELECT 1 FROM browser_sessions op WHERE op.token_hash=$2 AND op.operator_epoch=$3 AND op.created_at>now()-interval '24 hours' AND op.last_seen_at>now()-interval '12 hours'))) ORDER BY CASE WHEN m.role='speaker' AND $3::text IS NOT NULL THEN 0 ELSE 1 END,m.created_at LIMIT 1",
    )
    .bind(room_id)
    .bind(digest)
    .bind(access.security.operator_epoch.as_deref())
    .fetch_optional(&access.pool)
    .await {
        Ok(Some(row)) => row,
        Ok(None) => return reject(StatusCode::NOT_FOUND),
        Err(_) => return reject(StatusCode::SERVICE_UNAVAILABLE),
    };
    if row.get::<String, _>("member_status") != "active" {
        return reject(StatusCode::NOT_FOUND);
    }
    let role: String = row.get("role");
    let operator = access.security.operator_enabled() && row.get::<bool, _>("is_operator");
    if role == "speaker" && !operator {
        return reject(StatusCode::NOT_FOUND);
    }
    let current: i64 = row.get("sequence");
    let last_sequence = after.unwrap_or(current);
    if last_sequence < 0 || last_sequence > current {
        return reject(StatusCode::BAD_REQUEST);
    }
    let staff = role == "speaker" || role == "ta";
    let pool = access.pool;
    let session_hash = hash(presented.as_bytes());
    let expected_epoch = if operator {
        access.security.operator_epoch.clone()
    } else {
        None
    };
    let permits = CONNECTIONS.get_or_init(|| Arc::new(Semaphore::new(MAX_CONNECTIONS)));
    let permit = match permits.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return reject(StatusCode::SERVICE_UNAVAILABLE),
    };
    upgrade.on_upgrade(move |socket| {
        stream(
            socket,
            pool,
            LiveConnection {
                room_id,
                session_hash,
                expected_epoch,
                role,
                staff,
                sequence: last_sequence,
                _permit: permit,
            },
        )
    })
}

fn parse_after(query: Option<&str>) -> Option<Option<i64>> {
    let mut after = None;
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
        if key == "after" && after.is_none() {
            let sequence = value.parse::<i64>().ok()?;
            if sequence < 0 {
                return None;
            }
            after = Some(sequence);
        } else {
            return None;
        }
    }
    Some(after)
}

struct LiveConnection {
    room_id: Uuid,
    session_hash: Vec<u8>,
    expected_epoch: Option<String>,
    role: String,
    staff: bool,
    sequence: i64,
    _permit: OwnedSemaphorePermit,
}

async fn stream(
    mut socket: axum::extract::ws::WebSocket,
    pool: PgPool,
    connection: LiveConnection,
) {
    let LiveConnection {
        room_id,
        session_hash,
        expected_epoch,
        role,
        staff,
        mut sequence,
        _permit,
    } = connection;
    use axum::extract::ws::Message;
    let mut ticker = interval(POLL_INTERVAL);
    let mut last_ping = Instant::now();
    let mut pong_deadline = None;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let started = Instant::now();
                if pong_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    close(&mut socket, 1001, "heartbeat timeout").await;
                    return;
                }
                if last_ping.elapsed() >= HEARTBEAT_INTERVAL && pong_deadline.is_none() {
                    if timeout(SEND_TIMEOUT, socket.send(Message::Ping(Vec::new().into()))).await.is_err() {
                        return;
                    }
                    last_ping = Instant::now();
                    pong_deadline = Some(last_ping + PONG_TIMEOUT);
                    tracing::debug!(room_id=%room_id, role=%role, "live heartbeat sent");
                }
                let auth = sqlx::query(
                    "SELECT r.status AS room_status,m.status AS member_status,m.role FROM rooms r JOIN memberships m ON m.room_id=r.id JOIN browser_sessions s ON s.token_hash=$2 AND s.created_at>now()-interval '24 hours' AND s.last_seen_at>now()-interval '12 hours' WHERE r.id=$1 AND (m.session_id=s.id OR (m.role='speaker' AND $3::text IS NOT NULL AND s.operator_epoch=$3)) ORDER BY CASE WHEN m.role='speaker' AND $3::text IS NOT NULL THEN 0 ELSE 1 END,m.created_at LIMIT 1"
                ).bind(room_id).bind(&session_hash).bind(expected_epoch.as_deref()).fetch_optional(&pool).await;
                let auth = match auth {
                    Ok(Some(row)) if row.get::<String,_>("member_status") == "active" && row.get::<String,_>("role") == role => row,
                    Ok(_) => { close(&mut socket, 1008, "room access revoked").await; return; }
                    Err(_) => { close(&mut socket, 1011, "storage unavailable").await; return; }
                };
                let room_status: String = auth.get("room_status");
                let result = sqlx::query(
                    "SELECT event_id,event_type,schema_version,sequence,occurred_at,payload,visibility FROM room_events WHERE room_id=$1 AND sequence>$2 ORDER BY sequence LIMIT $3"
                )
                .bind(room_id).bind(sequence).bind(MAX_EVENTS_PER_POLL + 1).fetch_all(&pool).await;
                let mut rows = match result {
                    Ok(rows) => rows,
                    Err(_) => { close(&mut socket, 1011, "storage unavailable").await; return; }
                };
                rows.truncate(MAX_EVENTS_PER_POLL as usize);
                for row in rows {
                    sequence = row.get("sequence");
                    let visibility: String = row.get("visibility");
                    if visibility == "speaker" && !staff { continue; }
                    let payload: Value = row.get("payload");
                    let event = json!({
                        "type": row.get::<String,_>("event_type"),
                        "schema_version": row.get::<i32,_>("schema_version"),
                        "event_id": row.get::<Uuid,_>("event_id"),
                        "room_id": room_id,
                        "sequence": sequence,
                        "occurred_at": row.get::<chrono::DateTime<chrono::Utc>,_>("occurred_at"),
                        "resources": payload.get("resources").cloned().unwrap_or_else(|| json!(["state"])),
                    });
                    let send_started = Instant::now();
                    if timeout(SEND_TIMEOUT, socket.send(Message::Text(event.to_string().into()))).await.is_err() {
                        tracing::debug!(room_id=%room_id, role=%role, "live websocket send timed out");
                        return;
                    }
                    tracing::debug!(room_id=%room_id, role=%role, send_ms=send_started.elapsed().as_millis(), "live event sent");
                }
                tracing::debug!(room_id=%room_id, role=%role, poll_ms=started.elapsed().as_millis(), "live event poll completed");
                if room_status == "ended" {
                    close(&mut socket, 1000, "room ended").await;
                    return;
                }
            }
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(Message::Ping(payload))) => {
                    let started = Instant::now();
                    if timeout(SEND_TIMEOUT, socket.send(Message::Pong(payload))).await.is_err() { return; }
                    tracing::debug!(room_id=%room_id, receive_ms=started.elapsed().as_millis(), "live websocket ping handled");
                }
                Some(Ok(Message::Text(_) | Message::Binary(_))) => {
                    close(&mut socket, 1008, "read-only connection").await;
                    return;
                }
                Some(Ok(Message::Pong(_))) => {
                    tracing::debug!(room_id=%room_id, role=%role, receive_ms=last_ping.elapsed().as_millis(), "live heartbeat received");
                    pong_deadline = None;
                    last_ping = Instant::now();
                }
            }
        }
    }
}

async fn close(socket: &mut axum::extract::ws::WebSocket, code: u16, reason: &'static str) {
    use axum::extract::ws::{CloseFrame, Message};
    let _ = timeout(
        SEND_TIMEOUT,
        socket.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::parse_after;

    #[test]
    fn accepts_only_one_nonnegative_sequence_cursor() {
        assert_eq!(parse_after(None), Some(None));
        assert_eq!(parse_after(Some("")), Some(None));
        assert_eq!(parse_after(Some("after=42")), Some(Some(42)));
        assert_eq!(parse_after(Some("after=0")), Some(Some(0)));
        assert_eq!(parse_after(Some("after=-1")), None);
        assert_eq!(parse_after(Some("after=1&after=2")), None);
        assert_eq!(parse_after(Some("cursor=1")), None);
        assert_eq!(parse_after(Some("after=not-a-number")), None);
        assert_eq!(parse_after(Some("after=9223372036854775808")), None);
    }
}

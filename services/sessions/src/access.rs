//! Room create/join, membership, and speaker/TA access.
//! Transactions lock the browser session, then the room. Room changes, replay
//! receipts, and invalidations commit together. Token-bearing URIs are not logged.

use crate::security::{Security, equal, hash, token};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use cookie::{Cookie, SameSite};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};
use uuid::Uuid;

const MAX_BODY: usize = 16 * 1024;

#[derive(Clone)]
pub struct Access {
    pub pool: PgPool,
    pub security: Security,
}

pub struct ApiError(pub StatusCode, pub &'static str, pub &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        reply(
            self.0,
            json!({"code":self.1,"message":self.2,"request_id":Uuid::new_v4()}),
        )
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "Storage unavailable",
        )
    }
}

type Result<T> = std::result::Result<T, ApiError>;

fn invalid() -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "Invalid request",
    )
}

fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "not_found", "Resource not found")
}

fn forbidden() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "forbidden", "Permission denied")
}

fn conflict(code: &'static str, message: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code, message)
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn h<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    Cookie::split_parse(h(headers, "cookie")?)
        .filter_map(std::result::Result::ok)
        .find(|cookie| cookie.name() == "sanscue_session")
        .map(|cookie| cookie.value().to_owned())
}

fn id(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).map_err(|_| missing())
}

fn operator(session: &PgRow, security: &Security) -> bool {
    security.operator_enabled()
        && session.get::<Option<String>, _>("operator_epoch") == security.operator_epoch
}

fn parts(path: &str) -> Vec<&str> {
    path.trim_matches('/').split('/').collect()
}

fn request_target(path: &str) -> String {
    hex::encode(hash(format!("POST {path}").as_bytes()))
}

fn request_fingerprint(
    security: &Security,
    path: &str,
    body: &Value,
    login: bool,
) -> Result<Vec<u8>> {
    let encoded = serde_json::to_vec(body).map_err(|_| invalid())?;
    let mut material = Vec::with_capacity(path.len() + encoded.len() + 1);
    material.extend_from_slice(path.as_bytes());
    material.push(0);
    material.extend_from_slice(&encoded);
    Ok(security.fingerprint(&material, login))
}

fn redact_links(mut body: Value) -> Value {
    let mut redacted = false;
    for field in ["join_url", "invite_url"] {
        if body.get(field).is_some() {
            body[field] = Value::Null;
            redacted = true;
        }
    }
    if redacted {
        body["link_unavailable"] = json!(true);
    }
    body
}

struct Context {
    room_id: Option<Uuid>,
    member: Option<PgRow>,
    invitation: Option<PgRow>,
}

impl Context {
    fn empty() -> Self {
        Self {
            room_id: None,
            member: None,
            invitation: None,
        }
    }
}

pub fn router(access: Access) -> Router {
    Router::new()
        .route("/bootstrap", get(bootstrap))
        .route("/operator/login", post(mutate))
        .route("/rooms", post(mutate))
        .route("/join/{token}", post(mutate))
        .route("/invitations/{token}/redeem", post(mutate))
        .route("/rooms/{room}/state", get(read))
        .route("/rooms/{room}/invitations", get(read).post(mutate))
        .route("/rooms/{room}/memberships", get(read))
        .route(
            "/rooms/{room}/invitations/{invitation}/revoke",
            post(mutate),
        )
        .route(
            "/rooms/{room}/memberships/{membership}/revoke",
            post(mutate),
        )
        .route("/rooms/{room}/join-link/rotate", post(mutate))
        .route("/rooms/{room}/join-link/revoke", post(mutate))
        .route("/rooms/{room}/end", post(mutate))
        .route("/rooms/{room}/bee/bind", post(mutate))
        .route("/rooms/{room}/bee/unbind", post(mutate))
        .fallback(|| async { missing() })
        .with_state(access)
}

async fn bootstrap(State(access): State<Access>, headers: HeaderMap) -> Result<Response> {
    if access.security.origin.is_none() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "Browser origin not configured",
        ));
    }
    let mut tx = access.pool.begin().await?;
    let (csrf, new_token) = match load_session(&mut tx, &headers).await {
        Ok(session) => (session.get::<String, _>("csrf"), None),
        Err(error) if error.0 == StatusCode::UNAUTHORIZED => {
            let issued = token();
            let csrf = token();
            sqlx::query("INSERT INTO browser_sessions(id,token_hash,csrf) VALUES($1,$2,$3)")
                .bind(Uuid::new_v4())
                .bind(hash(issued.as_bytes()))
                .bind(&csrf)
                .execute(&mut *tx)
                .await?;
            (csrf, Some(issued))
        }
        Err(error) => return Err(error),
    };
    tx.commit().await?;
    let mut response = reply(StatusCode::OK, json!({"csrf_token":csrf}));
    if let Some(issued) = new_token {
        let cookie = session_cookie(&issued, &access.security)?;
        response.headers_mut().insert(
            header::SET_COOKIE,
            cookie.to_string().parse().map_err(|_| invalid())?,
        );
    }
    Ok(response)
}

fn session_cookie(issued: &str, security: &Security) -> Result<Cookie<'static>> {
    let http = security
        .origin
        .as_ref()
        .is_some_and(|origin| origin.starts_with("http://"));
    Ok(Cookie::build(("sanscue_session", issued.to_owned()))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(!(http && security.insecure_loopback))
        .max_age(cookie::time::Duration::seconds(86400))
        .build())
}

async fn load_session(tx: &mut Transaction<'_, Postgres>, headers: &HeaderMap) -> Result<PgRow> {
    let presented = cookie_token(headers).ok_or(ApiError(
        StatusCode::UNAUTHORIZED,
        "session_required",
        "Bootstrap required",
    ))?;
    let row = sqlx::query(
        "SELECT * FROM browser_sessions WHERE token_hash=$1 AND created_at>now()-interval '24 hours' AND last_seen_at>now()-interval '12 hours' FOR UPDATE",
    )
    .bind(hash(presented.as_bytes()))
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ApiError(
        StatusCode::UNAUTHORIZED,
        "session_required",
        "Bootstrap required",
    ))?;
    sqlx::query("UPDATE browser_sessions SET last_seen_at=now() WHERE id=$1")
        .bind(row.get::<Uuid, _>("id"))
        .execute(&mut **tx)
        .await?;
    Ok(row)
}

async fn room(tx: &mut Transaction<'_, Postgres>, room_id: Uuid) -> Result<PgRow> {
    sqlx::query("SELECT * FROM rooms WHERE id=$1 FOR UPDATE")
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(missing)
}

async fn lock_membership(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    session_id: Uuid,
    is_operator: bool,
) -> Result<Option<PgRow>> {
    Ok(sqlx::query(
        "SELECT * FROM memberships WHERE room_id=$1 AND (session_id=$2 OR (role='speaker' AND $3)) ORDER BY CASE WHEN role='speaker' AND $3 THEN 0 ELSE 1 END, CASE WHEN status='active' THEN 0 ELSE 1 END, created_at LIMIT 1 FOR UPDATE",
    )
    .bind(room_id)
    .bind(session_id)
    .bind(is_operator)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn membership(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    session: &PgRow,
    security: &Security,
) -> Result<PgRow> {
    let is_operator = operator(session, security);
    let row = lock_membership(tx, room_id, session.get("id"), is_operator)
        .await?
        .ok_or_else(missing)?;
    if row.get::<String, _>("status") != "active"
        || (row.get::<String, _>("role") == "speaker" && !is_operator)
    {
        return Err(missing());
    }
    touch(tx, row.get("id")).await
}

async fn touch(tx: &mut Transaction<'_, Postgres>, membership_id: Uuid) -> Result<PgRow> {
    Ok(
        sqlx::query("UPDATE memberships SET last_seen_at=now() WHERE id=$1 RETURNING *")
            .bind(membership_id)
            .fetch_one(&mut **tx)
            .await?,
    )
}

fn require_speaker(member: &PgRow) -> Result<()> {
    if member.get::<String, _>("role") == "speaker" {
        Ok(())
    } else {
        Err(forbidden())
    }
}

fn snapshot(room_row: &PgRow, member: &PgRow) -> Value {
    let role = member.get::<String, _>("role");
    let mut value = json!({
        "room": {
            "id": room_row.get::<Uuid, _>("id"),
            "title": room_row.get::<String, _>("title"),
            "status": room_row.get::<String, _>("status"),
        },
        "membership": {"id": member.get::<Uuid, _>("id"), "role": role},
        "revision": room_row.get::<i64, _>("sequence"),
        "sequence": room_row.get::<i64, _>("sequence"),
        "active_question": null,
        "my_response": null,
        "dashboard": null,
        "bee": null,
    });
    if role != "audience" {
        value["dashboard"] = json!({
            "respondents": 0,
            "counts": {"clear": 0, "partly_clear": 0, "need_help": 0},
            "percentages": {"clear": null, "partly_clear": null, "need_help": null},
        });
        value["bee"] = json!({
            "binding_id": null,
            "status": "unbound",
            "has_gaps": false,
            "pending_command_id": null,
            "pending_command_revision": null,
            "latest_command": null,
            "connectivity_revision": 0,
        });
    }
    if role == "speaker" {
        value["join_link_enabled"] = json!(
            room_row.get::<Option<Vec<u8>>, _>("join_hash").is_some()
                && room_row.get::<String, _>("status") == "open"
        );
    }
    value
}

async fn bee_snapshot(tx: &mut Transaction<'_, Postgres>, room_id: Uuid) -> Result<Value> {
    if let Some(row) = sqlx::query("SELECT status FROM bee_room_status WHERE room_id=$1")
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await?
    {
        return Ok(row.get::<Value, _>("status"));
    }
    if let Some(row) = sqlx::query(
        "SELECT conversation_id,status,revision FROM bee_room_bindings WHERE room_id=$1",
    )
    .bind(room_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(
            json!({"binding_id":row.get::<Uuid,_>("conversation_id"),"status":row.get::<String,_>("status"),"has_gaps":false,"pending_command_id":null,"pending_command_revision":row.get::<i64,_>("revision"),"latest_command":null,"connectivity_revision":0}),
        );
    }
    Ok(
        json!({"binding_id":null,"status":"unbound","has_gaps":false,"pending_command_id":null,"pending_command_revision":null,"latest_command":null,"connectivity_revision":0}),
    )
}

async fn change(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    speaker_only: bool,
) -> Result<()> {
    let sequence: i64 =
        sqlx::query_scalar("UPDATE rooms SET sequence=sequence+1 WHERE id=$1 RETURNING sequence")
            .bind(room_id)
            .fetch_one(&mut **tx)
            .await?;
    let payload = if speaker_only {
        json!({"resources":["memberships","invitations"]})
    } else {
        json!({"resources":["state"]})
    };
    sqlx::query(
        "INSERT INTO room_events(event_id,room_id,sequence,payload,visibility) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(Uuid::new_v4())
    .bind(room_id)
    .bind(sequence)
    .bind(payload)
    .bind(if speaker_only { "speaker" } else { "all" })
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn add_member(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    session_id: Uuid,
    role: &str,
) -> Result<PgRow> {
    let n: i64 = sqlx::query_scalar("SELECT count(*)+1 FROM memberships WHERE room_id=$1")
        .bind(room_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(sqlx::query(
        "INSERT INTO memberships(id,room_id,session_id,role,label,last_seen_at) VALUES($1,$2,$3,$4,$5,now()) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(room_id)
    .bind(session_id)
    .bind(role)
    .bind(format!("Participant {n}"))
    .fetch_one(&mut **tx)
    .await?)
}

fn check_origin(security: &Security, headers: &HeaderMap) -> Result<()> {
    if security.origin.is_some() && h(headers, "origin") == security.origin.as_deref() {
        Ok(())
    } else {
        Err(ApiError(
            StatusCode::FORBIDDEN,
            "origin_denied",
            "Origin denied",
        ))
    }
}

fn json_content_type(headers: &HeaderMap) -> bool {
    h(headers, "content-type").is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
    })
}

fn check_shape(path: &str, body: &Value) -> Result<()> {
    let object = body.as_object().ok_or_else(invalid)?;
    if path.ends_with("/bee/bind") {
        let allowed = ["conversation_id", "source_conversation_id"];
        if !object.contains_key("conversation_id")
            || object.keys().any(|k| !allowed.contains(&k.as_str()))
            || !body["conversation_id"].is_string()
            || body
                .get("source_conversation_id")
                .is_some_and(|v| !v.is_null() && !v.is_string())
        {
            return Err(invalid());
        }
        return Ok(());
    }
    let expected = if path == "/operator/login" {
        Some("credential")
    } else if path == "/rooms" {
        Some("title")
    } else {
        None
    };
    if object.len() != usize::from(expected.is_some())
        || expected.is_some_and(|key| !object.get(key).is_some_and(Value::is_string))
    {
        return Err(invalid());
    }
    Ok(())
}

fn check_csrf(headers: &HeaderMap, session: &PgRow) -> Result<()> {
    if equal(
        h(headers, "x-csrf-token").unwrap_or("").as_bytes(),
        session.get::<String, _>("csrf").as_bytes(),
    ) {
        Ok(())
    } else {
        Err(ApiError(
            StatusCode::FORBIDDEN,
            "csrf_failed",
            "CSRF validation failed",
        ))
    }
}

/// `Ok(true)` means the credential was rejected and the failure counter must be committed.
async fn reject_bad_login(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    session: &PgRow,
    body: &Value,
) -> Result<bool> {
    if !security.operator_enabled() {
        return Err(forbidden());
    }
    let session_id: Uuid = session.get("id");
    let limited: bool = sqlx::query_scalar(
        "SELECT login_failures >= 5 AND failure_window > now() - interval '15 minutes' FROM browser_sessions WHERE id=$1",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await?;
    if limited {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Try again later",
        ));
    }
    let credential = body["credential"].as_str().ok_or_else(invalid)?;
    let configured = security.operator_hash.as_deref().ok_or_else(forbidden)?;
    if equal(&hash(credential.as_bytes()), configured) {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE browser_sessions SET login_failures=CASE WHEN failure_window<now()-interval '15 minutes' THEN 1 ELSE login_failures+1 END, failure_window=CASE WHEN failure_window<now()-interval '15 minutes' THEN now() ELSE failure_window END WHERE id=$1",
    )
    .bind(session_id)
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

fn allowed_when_ended(path: &str) -> bool {
    path.ends_with("/revoke") || path.ends_with("/end") || path.ends_with("/bee/unbind")
}

fn ended_block(path: &str) -> ApiError {
    if path.starts_with("/join/") || path.starts_with("/invitations/") {
        missing()
    } else {
        conflict("room_ended", "Room has ended")
    }
}

async fn authorize_replay(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    session: &PgRow,
    path: &str,
    prior: &PgRow,
) -> Result<()> {
    if path == "/operator/login" {
        return Ok(());
    }
    if path == "/rooms" {
        return if operator(session, security) {
            Ok(())
        } else {
            Err(forbidden())
        };
    }
    let Some(room_id) = prior.get::<Option<Uuid>, _>("room_id") else {
        return Err(missing());
    };
    room(tx, room_id).await?;
    let is_operator = operator(session, security);
    let member = lock_membership(tx, room_id, session.get("id"), is_operator).await?;
    if path.starts_with("/rooms/") {
        let member = member.ok_or_else(missing)?;
        if member.get::<String, _>("status") != "active"
            || (member.get::<String, _>("role") == "speaker" && !is_operator)
        {
            return Err(missing());
        }
        return require_speaker(&member);
    }
    let Some(member) = member else {
        return Err(missing());
    };
    if member.get::<String, _>("status") != "active"
        || (member.get::<String, _>("role") == "speaker" && !is_operator)
    {
        Err(missing())
    } else {
        Ok(())
    }
}

async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    session: &PgRow,
    path: &str,
    key: Uuid,
    fingerprint: &[u8],
    login: bool,
) -> Result<Option<Response>> {
    let session_id: Uuid = session.get("id");
    let target = request_target(path);
    let prior =
        sqlx::query("SELECT * FROM idempotency WHERE session_id=$1 AND target=$2 AND key=$3")
            .bind(session_id)
            .bind(&target)
            .bind(key)
            .fetch_optional(&mut **tx)
            .await?;
    let Some(prior) = prior else {
        return Ok(None);
    };
    if login && prior.get::<Option<String>, _>("epoch") != security.operator_epoch {
        sqlx::query("DELETE FROM idempotency WHERE session_id=$1 AND target=$2 AND key=$3")
            .bind(session_id)
            .bind(&target)
            .bind(key)
            .execute(&mut **tx)
            .await?;
        return Ok(None);
    }
    if !equal(fingerprint, &prior.get::<Vec<u8>, _>("fingerprint")) {
        return Err(conflict(
            "idempotency_conflict",
            "Key already used with different request",
        ));
    }
    authorize_replay(tx, security, session, path, &prior).await?;
    let status = u16::try_from(prior.get::<i32, _>("status")).map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "Storage unavailable",
        )
    })?;
    let status = StatusCode::from_u16(status).map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "Storage unavailable",
        )
    })?;
    Ok(Some(reply(status, prior.get("body"))))
}

async fn resolve(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    session: &PgRow,
    path: &str,
) -> Result<Context> {
    if path == "/operator/login" {
        return Ok(Context::empty());
    }
    if path == "/rooms" {
        return if operator(session, security) {
            Ok(Context::empty())
        } else {
            Err(forbidden())
        };
    }
    let p = parts(path);
    if p.first() == Some(&"rooms") {
        if p.len() < 3 {
            return Err(missing());
        }
        let room_id = id(p[1])?;
        room(tx, room_id).await?;
        let member = membership(tx, room_id, session, security).await?;
        require_speaker(&member)?;
        return Ok(Context {
            room_id: Some(room_id),
            member: Some(member),
            invitation: None,
        });
    }
    if p.first() == Some(&"join") {
        return resolve_join(tx, security, session, &p).await;
    }
    if p.first() == Some(&"invitations") {
        return resolve_redeem(tx, security, session, &p).await;
    }
    Err(missing())
}

async fn resolve_join(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    session: &PgRow,
    p: &[&str],
) -> Result<Context> {
    if p.len() != 2 {
        return Err(missing());
    }
    let digest = hash(p[1].as_bytes());
    let found = sqlx::query("SELECT id FROM rooms WHERE join_hash=$1")
        .bind(&digest)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(missing)?;
    let room_id: Uuid = found.get("id");
    let locked = room(tx, room_id).await?;
    if !locked
        .get::<Option<Vec<u8>>, _>("join_hash")
        .is_some_and(|value| equal(&value, &digest))
    {
        return Err(missing());
    }
    let is_operator = operator(session, security);
    let member = lock_membership(tx, room_id, session.get("id"), is_operator).await?;
    if let Some(ref row) = member
        && (row.get::<String, _>("status") == "revoked"
            || (row.get::<String, _>("role") == "speaker" && !is_operator))
    {
        return Err(missing());
    }
    Ok(Context {
        room_id: Some(room_id),
        member,
        invitation: None,
    })
}

async fn resolve_redeem(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    session: &PgRow,
    p: &[&str],
) -> Result<Context> {
    if p.len() != 3 || p[2] != "redeem" {
        return Err(missing());
    }
    let digest = hash(p[1].as_bytes());
    let located = sqlx::query("SELECT room_id FROM invitations WHERE token_hash=$1")
        .bind(&digest)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(missing)?;
    let room_id: Uuid = located.get("room_id");
    room(tx, room_id).await?;
    let Some(invitation) = sqlx::query(
        "SELECT *, (expires_at <= now()) AS expired FROM invitations WHERE token_hash=$1 FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Err(missing());
    };
    if !equal(&digest, &invitation.get::<Vec<u8>, _>("token_hash"))
        || invitation.get::<String, _>("status") == "revoked"
        || invitation.get::<bool, _>("expired")
    {
        return Err(missing());
    }
    let is_operator = operator(session, security);
    let session_id: Uuid = session.get("id");
    let member = lock_membership(tx, room_id, session_id, is_operator).await?;
    if member
        .as_ref()
        .is_some_and(|row| row.get::<String, _>("role") == "speaker")
    {
        return Err(conflict("invite_used", "Invitation cannot be used"));
    }
    let same_grant = member.as_ref().is_some_and(|row| {
        row.get::<String, _>("status") == "active"
            && row.get::<Option<Uuid>, _>("current_grant_id")
                == invitation.get::<Option<Uuid>, _>("grant_id")
    });
    if invitation.get::<String, _>("status") == "redeemed"
        && (invitation.get::<Option<Uuid>, _>("redeemer_session_id") != Some(session_id)
            || !same_grant)
    {
        return Err(conflict("invite_used", "Invitation already used"));
    }
    Ok(Context {
        room_id: Some(room_id),
        member,
        invitation: Some(invitation),
    })
}

struct Outcome {
    status: StatusCode,
    body: Value,
    room_id: Option<Uuid>,
}

async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    path: &str,
    body: &Value,
    session_id: Uuid,
    ctx: Context,
) -> Result<Outcome> {
    if path == "/operator/login" {
        sqlx::query("UPDATE browser_sessions SET operator_epoch=$2, login_failures=0 WHERE id=$1")
            .bind(session_id)
            .bind(&security.operator_epoch)
            .execute(&mut **tx)
            .await?;
        return Ok(Outcome {
            status: StatusCode::OK,
            body: json!({"operator": true}),
            room_id: None,
        });
    }
    if path == "/rooms" {
        return create_room(tx, body, session_id).await;
    }
    let room_id = ctx.room_id.ok_or_else(missing)?;
    let p = parts(path);
    let body = if p.first() == Some(&"join") {
        join_room(tx, room_id, session_id, ctx.member).await?
    } else if p.first() == Some(&"invitations") {
        redeem(tx, room_id, session_id, ctx).await?
    } else if p.get(2) == Some(&"invitations") && p.len() == 3 {
        return issue_invitation(tx, room_id).await;
    } else if p.get(2) == Some(&"invitations") && p.len() == 5 && p[4] == "revoke" {
        revoke_invitation(tx, room_id, id(p[3])?).await?
    } else if p.get(2) == Some(&"memberships") && p.len() == 5 && p[4] == "revoke" {
        revoke_membership(tx, room_id, id(p[3])?).await?
    } else if p.get(2) == Some(&"join-link") && p.len() == 4 && p[3] == "rotate" {
        rotate_join(tx, room_id).await?
    } else if p.get(2) == Some(&"join-link") && p.len() == 4 && p[3] == "revoke" {
        revoke_join(tx, room_id).await?
    } else if p.get(2) == Some(&"end") && p.len() == 3 {
        end_room(tx, room_id).await?
    } else if p.get(2) == Some(&"bee") && p.len() == 4 && p[3] == "bind" {
        bind_bee(tx, room_id, body).await?
    } else if p.get(2) == Some(&"bee") && p.len() == 4 && p[3] == "unbind" {
        unbind_bee(tx, room_id).await?
    } else {
        return Err(missing());
    };
    Ok(Outcome {
        status: StatusCode::OK,
        body,
        room_id: Some(room_id),
    })
}

async fn bind_bee(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    body: &Value,
) -> Result<Value> {
    let conversation = Uuid::parse_str(body["conversation_id"].as_str().ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    if conversation.is_nil() {
        return Err(invalid());
    }
    let source = body["source_conversation_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if source.is_some_and(|s| s.len() > 256) {
        return Err(invalid());
    }
    if let Some(old) =
        sqlx::query("SELECT status FROM bee_room_bindings WHERE room_id=$1 FOR UPDATE")
            .bind(room_id)
            .fetch_optional(&mut **tx)
            .await?
        && old.get::<String, _>("status") != "unbound"
    {
        return Err(conflict(
            "bee_already_bound",
            "Unbind the current conversation before changing it",
        ));
    }
    let rev: i64 = sqlx::query_scalar("SELECT sequence+1 FROM rooms WHERE id=$1")
        .bind(room_id)
        .fetch_one(&mut **tx)
        .await?;
    let command_id = Uuid::new_v4();
    let speaker_session: Uuid = sqlx::query_scalar("SELECT session_id FROM memberships WHERE room_id=$1 AND role='speaker' AND status='active'")
        .bind(room_id).fetch_one(&mut **tx).await?;
    let payload = json!({"conversation_id":conversation,"source_conversation_id":source,"room_id":room_id,"session_id":speaker_session,"revision":rev});
    sqlx::query("INSERT INTO bee_room_bindings(room_id,conversation_id,source_conversation_id,status,revision) VALUES($1,$2,$3,'binding',$4) ON CONFLICT(room_id) DO UPDATE SET conversation_id=EXCLUDED.conversation_id,source_conversation_id=EXCLUDED.source_conversation_id,status='binding',revision=EXCLUDED.revision,updated_at=now()")
        .bind(room_id).bind(conversation).bind(source).bind(rev).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO bee_command_outbox(command_id,room_id,conversation_id,command_type,revision,payload) VALUES($1,$2,$3,'bind',$4,$5)").bind(command_id).bind(room_id).bind(conversation).bind(rev).bind(payload).execute(&mut **tx).await?;
    change(tx, room_id, false).await?;
    Ok(json!({"status":"binding","command_id":command_id,"revision":rev}))
}

async fn unbind_bee(tx: &mut Transaction<'_, Postgres>, room_id: Uuid) -> Result<Value> {
    let row = sqlx::query(
        "SELECT conversation_id,status FROM bee_room_bindings WHERE room_id=$1 FOR UPDATE",
    )
    .bind(room_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(missing)?;
    let conversation: Uuid = row.get("conversation_id");
    let state: String = row.get("status");
    if state == "unbound" {
        return Ok(json!({"status":"unbound"}));
    }
    let rev: i64 = sqlx::query_scalar("SELECT sequence+1 FROM rooms WHERE id=$1")
        .bind(room_id)
        .fetch_one(&mut **tx)
        .await?;
    let command_id = Uuid::new_v4();
    let speaker_session: Uuid = sqlx::query_scalar("SELECT session_id FROM memberships WHERE room_id=$1 AND role='speaker' AND status='active'")
        .bind(room_id).fetch_one(&mut **tx).await?;
    let payload = json!({"conversation_id":conversation,"room_id":room_id,"session_id":speaker_session,"revision":rev});
    sqlx::query("UPDATE bee_room_bindings SET status='unbinding',revision=$2,updated_at=now() WHERE room_id=$1").bind(room_id).bind(rev).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO bee_command_outbox(command_id,room_id,conversation_id,command_type,revision,payload) VALUES($1,$2,$3,'unbind',$4,$5)").bind(command_id).bind(room_id).bind(conversation).bind(rev).bind(payload).execute(&mut **tx).await?;
    change(tx, room_id, false).await?;
    Ok(json!({"status":"unbinding","command_id":command_id,"revision":rev}))
}

async fn create_room(
    tx: &mut Transaction<'_, Postgres>,
    body: &Value,
    session_id: Uuid,
) -> Result<Outcome> {
    let title = body["title"].as_str().unwrap_or("").trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err(invalid());
    }
    let room_id = Uuid::new_v4();
    let issued = token();
    sqlx::query("INSERT INTO rooms(id,title,join_hash) VALUES($1,$2,$3)")
        .bind(room_id)
        .bind(title)
        .bind(hash(issued.as_bytes()))
        .execute(&mut **tx)
        .await?;
    let member = add_member(tx, room_id, session_id, "speaker").await?;
    change(tx, room_id, false).await?;
    let state = snapshot(&room(tx, room_id).await?, &member);
    Ok(Outcome {
        status: StatusCode::CREATED,
        body: json!({"state": state, "join_url": format!("/join/{issued}")}),
        room_id: Some(room_id),
    })
}

async fn join_room(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    session_id: Uuid,
    member: Option<PgRow>,
) -> Result<Value> {
    let member = if let Some(member) = member {
        touch(tx, member.get("id")).await?
    } else {
        let member = add_member(tx, room_id, session_id, "audience").await?;
        change(tx, room_id, true).await?;
        member
    };
    Ok(json!({"state": snapshot(&room(tx, room_id).await?, &member)}))
}

async fn redeem(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    session_id: Uuid,
    ctx: Context,
) -> Result<Value> {
    let invitation = ctx.invitation.ok_or_else(missing)?;
    if invitation.get::<String, _>("status") != "pending" {
        return Err(conflict("invite_used", "Invitation already used"));
    }
    let member = if let Some(member) = ctx.member {
        member
    } else {
        add_member(tx, room_id, session_id, "audience").await?
    };
    let grant = Uuid::new_v4();
    let member = sqlx::query(
        "UPDATE memberships SET role='ta', status='active', current_grant_id=$2, last_seen_at=now() WHERE id=$1 RETURNING *",
    )
    .bind(member.get::<Uuid, _>("id"))
    .bind(grant)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE invitations SET status='redeemed', membership_id=$2, grant_id=$3, redeemer_session_id=$4 WHERE id=$1",
    )
    .bind(invitation.get::<Uuid, _>("id"))
    .bind(member.get::<Uuid, _>("id"))
    .bind(grant)
    .bind(session_id)
    .execute(&mut **tx)
    .await?;
    change(tx, room_id, true).await?;
    Ok(json!({"state": snapshot(&room(tx, room_id).await?, &member)}))
}

async fn issue_invitation(tx: &mut Transaction<'_, Postgres>, room_id: Uuid) -> Result<Outcome> {
    let issued = token();
    let invitation =
        sqlx::query("INSERT INTO invitations(id,room_id,token_hash) VALUES($1,$2,$3) RETURNING *")
            .bind(Uuid::new_v4())
            .bind(room_id)
            .bind(hash(issued.as_bytes()))
            .fetch_one(&mut **tx)
            .await?;
    change(tx, room_id, true).await?;
    Ok(Outcome {
        status: StatusCode::CREATED,
        body: json!({
            "invitation_id": invitation.get::<Uuid, _>("id"),
            "invite_url": format!("/invite/{issued}"),
            "status": "pending",
            "expires_at": invitation.get::<DateTime<Utc>, _>("expires_at"),
        }),
        room_id: Some(room_id),
    })
}

async fn revoke_invitation(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    invitation_id: Uuid,
) -> Result<Value> {
    let invitation = sqlx::query("SELECT * FROM invitations WHERE id=$1 AND room_id=$2 FOR UPDATE")
        .bind(invitation_id)
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(missing)?;
    if invitation.get::<String, _>("status") != "revoked" {
        sqlx::query("UPDATE memberships SET status='revoked' WHERE id=$1 AND current_grant_id=$2")
            .bind(invitation.get::<Option<Uuid>, _>("membership_id"))
            .bind(invitation.get::<Option<Uuid>, _>("grant_id"))
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE invitations SET status='revoked' WHERE id=$1")
            .bind(invitation_id)
            .execute(&mut **tx)
            .await?;
        change(tx, room_id, true).await?;
    }
    Ok(json!({"invitation_id": invitation_id, "status": "revoked"}))
}

async fn revoke_membership(
    tx: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
    membership_id: Uuid,
) -> Result<Value> {
    let member = sqlx::query("SELECT * FROM memberships WHERE id=$1 AND room_id=$2 FOR UPDATE")
        .bind(membership_id)
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(missing)?;
    if member.get::<String, _>("role") == "speaker" {
        return Err(forbidden());
    }
    if member.get::<String, _>("status") != "revoked" {
        sqlx::query("UPDATE memberships SET status='revoked' WHERE id=$1")
            .bind(membership_id)
            .execute(&mut **tx)
            .await?;
        change(tx, room_id, true).await?;
    }
    Ok(json!({"membership_id": membership_id, "status": "revoked"}))
}

async fn rotate_join(tx: &mut Transaction<'_, Postgres>, room_id: Uuid) -> Result<Value> {
    let issued = token();
    sqlx::query("UPDATE rooms SET join_hash=$2 WHERE id=$1")
        .bind(room_id)
        .bind(hash(issued.as_bytes()))
        .execute(&mut **tx)
        .await?;
    change(tx, room_id, false).await?;
    Ok(json!({"join_url": format!("/join/{issued}"), "join_link_enabled": true}))
}

async fn revoke_join(tx: &mut Transaction<'_, Postgres>, room_id: Uuid) -> Result<Value> {
    let changed =
        sqlx::query("UPDATE rooms SET join_hash=NULL WHERE id=$1 AND join_hash IS NOT NULL")
            .bind(room_id)
            .execute(&mut **tx)
            .await?
            .rows_affected();
    if changed > 0 {
        change(tx, room_id, false).await?;
    }
    Ok(json!({"status": "revoked"}))
}

async fn end_room(tx: &mut Transaction<'_, Postgres>, room_id: Uuid) -> Result<Value> {
    let changed = sqlx::query(
        "UPDATE rooms SET status='ended', join_hash=NULL WHERE id=$1 AND status='open'",
    )
    .bind(room_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed > 0 {
        sqlx::query(
            "UPDATE invitations SET status='revoked' WHERE room_id=$1 AND status='pending'",
        )
        .bind(room_id)
        .execute(&mut **tx)
        .await?;
        change(tx, room_id, false).await?;
    }
    let revision: i64 = room(tx, room_id).await?.get("sequence");
    Ok(json!({
        "room_id": room_id,
        "status": "ended",
        "active_question": null,
        "revision": revision,
    }))
}

struct Receipt<'a> {
    session_id: Uuid,
    path: &'a str,
    key: Uuid,
    fingerprint: &'a [u8],
    login: bool,
}

async fn store_outcome(
    tx: &mut Transaction<'_, Postgres>,
    security: &Security,
    receipt: &Receipt<'_>,
    outcome: &Outcome,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO idempotency(session_id,target,key,fingerprint,status,body,epoch,room_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(receipt.session_id)
    .bind(request_target(receipt.path))
    .bind(receipt.key)
    .bind(receipt.fingerprint)
    .bind(i32::from(outcome.status.as_u16()))
    .bind(redact_links(outcome.body.clone()))
    .bind(if receipt.login {
        security.operator_epoch.as_deref()
    } else {
        None
    })
    .bind(outcome.room_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn mutate(
    State(access): State<Access>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Response> {
    if bytes.len() > MAX_BODY || !json_content_type(&headers) {
        return Err(invalid());
    }
    check_origin(&access.security, &headers)?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let path = uri.path();
    check_shape(path, &body)?;
    let key = Uuid::parse_str(h(&headers, "idempotency-key").ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    let login = path == "/operator/login";
    let fingerprint = request_fingerprint(&access.security, path, &body, login)?;
    let mut tx = access.pool.begin().await?;
    let session = load_session(&mut tx, &headers).await?;
    check_csrf(&headers, &session)?;
    if login && reject_bad_login(&mut tx, &access.security, &session, &body).await? {
        tx.commit().await?;
        return Err(forbidden());
    }
    if let Some(response) = replay(
        &mut tx,
        &access.security,
        &session,
        path,
        key,
        &fingerprint,
        login,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(response);
    }
    let ctx = resolve(&mut tx, &access.security, &session, path).await?;
    if let Some(room_id) = ctx.room_id {
        let row = room(&mut tx, room_id).await?;
        if row.get::<String, _>("status") == "ended" && !allowed_when_ended(path) {
            return Err(ended_block(path));
        }
    }
    let outcome = apply(
        &mut tx,
        &access.security,
        path,
        &body,
        session.get("id"),
        ctx,
    )
    .await?;
    store_outcome(
        &mut tx,
        &access.security,
        &Receipt {
            session_id: session.get("id"),
            path,
            key,
            fingerprint: &fingerprint,
            login,
        },
        &outcome,
    )
    .await?;
    tx.commit().await?;
    Ok(reply(outcome.status, outcome.body))
}

async fn read(
    State(access): State<Access>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<Response> {
    let p = parts(uri.path());
    if p.len() != 3 {
        return Err(missing());
    }
    let room_id = id(p[1])?;
    let mut tx = access.pool.begin().await?;
    let session = load_session(&mut tx, &headers).await?;
    let room_row = room(&mut tx, room_id).await?;
    let member = membership(&mut tx, room_id, &session, &access.security).await?;
    let body = if p[2] == "state" {
        let mut state = snapshot(&room_row, &member);
        if member.get::<String, _>("role") != "audience" {
            state["bee"] = bee_snapshot(&mut tx, room_id).await?;
        }
        state
    } else if p[2] == "memberships" || p[2] == "invitations" {
        require_speaker(&member)?;
        let (limit, after) = list_window(uri.query())?;
        list_page(&mut tx, p[2] == "memberships", room_id, after, limit).await?
    } else {
        return Err(missing());
    };
    tx.commit().await?;
    Ok(reply(StatusCode::OK, body))
}

fn list_window(query: Option<&str>) -> Result<(i64, Option<Uuid>)> {
    let mut limit = 50i64;
    let mut after = None;
    let mut saw_limit = false;
    let mut saw_after = false;
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
        match key.as_ref() {
            "limit" if !saw_limit => {
                saw_limit = true;
                limit = value.parse().map_err(|_| invalid())?;
                if !(1..=100).contains(&limit) {
                    return Err(invalid());
                }
            }
            "after" if !saw_after => {
                saw_after = true;
                after = Some(Uuid::parse_str(&value).map_err(|_| invalid())?);
            }
            _ => return Err(invalid()),
        }
    }
    Ok((limit, after))
}

async fn list_page(
    tx: &mut Transaction<'_, Postgres>,
    memberships: bool,
    room_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Value> {
    let cursor = match after {
        Some(cursor) => {
            let row = if memberships {
                sqlx::query("SELECT created_at FROM memberships WHERE id=$1 AND room_id=$2")
            } else {
                sqlx::query("SELECT created_at FROM invitations WHERE id=$1 AND room_id=$2")
            }
            .bind(cursor)
            .bind(room_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(invalid)?;
            Some((row.get::<DateTime<Utc>, _>("created_at"), cursor))
        }
        None => None,
    };
    let rows = fetch_page(
        tx,
        memberships,
        room_id,
        cursor.as_ref().map(|item| item.0),
        cursor.as_ref().map(|item| item.1),
        limit + 1,
    )
    .await?;
    let next = if rows.len() > limit as usize {
        Some(rows[limit as usize - 1].get::<Uuid, _>("id"))
    } else {
        None
    };
    let items: Vec<_> = rows
        .iter()
        .take(limit as usize)
        .map(|row| {
            if memberships {
                membership_item(row)
            } else {
                invitation_item(row)
            }
        })
        .collect();
    Ok(json!({"items": items, "next_cursor": next}))
}

async fn fetch_page(
    tx: &mut Transaction<'_, Postgres>,
    memberships: bool,
    room_id: Uuid,
    created_at: Option<DateTime<Utc>>,
    cursor: Option<Uuid>,
    limit: i64,
) -> Result<Vec<PgRow>> {
    let statement = if memberships {
        "SELECT * FROM memberships WHERE room_id=$1 AND ($2::timestamptz IS NULL OR (created_at,id)>($2,$3)) ORDER BY created_at,id LIMIT $4"
    } else {
        "SELECT *, (expires_at <= now()) AS expired FROM invitations WHERE room_id=$1 AND ($2::timestamptz IS NULL OR (created_at,id)>($2,$3)) ORDER BY created_at,id LIMIT $4"
    };
    Ok(sqlx::query(statement)
        .bind(room_id)
        .bind(created_at)
        .bind(cursor)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?)
}

fn membership_item(row: &PgRow) -> Value {
    json!({
        "membership_id": row.get::<Uuid, _>("id"),
        "label": row.get::<String, _>("label"),
        "role": row.get::<String, _>("role"),
        "status": row.get::<String, _>("status"),
        "last_seen_at": row.get::<Option<DateTime<Utc>>, _>("last_seen_at"),
        "current_grant_id": row.get::<Option<Uuid>, _>("current_grant_id"),
    })
}

fn invitation_item(row: &PgRow) -> Value {
    let status = row.get::<String, _>("status");
    let shown = if status == "pending" && row.get::<bool, _>("expired") {
        "expired".to_owned()
    } else {
        status
    };
    json!({
        "invitation_id": row.get::<Uuid, _>("id"),
        "status": shown,
        "membership_id": row.get::<Option<Uuid>, _>("membership_id"),
        "grant_id": row.get::<Option<Uuid>, _>("grant_id"),
        "expires_at": row.get::<DateTime<Utc>, _>("expires_at"),
    })
}

/// Private listener: authenticated readiness only. Delivery routes are later work.
/// Public routers never mount this path.
pub fn internal_router(access: Access) -> Router {
    Router::new()
        .route("/internal/v1/readyz", get(internal_ready))
        .route("/internal/v1/bee/commands", get(bee_commands))
        .route(
            "/internal/v1/bee/commands/{command}/ack",
            post(bee_command_ack),
        )
        .route("/internal/v1/bee/rooms/{room}/status", post(bee_status))
        .fallback(|| async { missing() })
        .with_state(access)
}

fn require_bee(headers: &HeaderMap, access: &Access) -> Result<()> {
    let Some(value) = h(headers, "authorization").and_then(|v| v.strip_prefix("Bearer ")) else {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "session_required",
            "Service authentication required",
        ));
    };
    if !credential_matches(&access.security.bee_credential, value) {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "session_required",
            "Service authentication required",
        ));
    }
    Ok(())
}

async fn bee_commands(State(access): State<Access>, headers: HeaderMap) -> Result<Response> {
    require_bee(&headers, &access)?;
    let rows=sqlx::query("SELECT command_id,room_id,conversation_id,command_type,revision,payload FROM bee_command_outbox WHERE delivered_at IS NULL ORDER BY created_at LIMIT 100").fetch_all(&access.pool).await?;
    let commands: Vec<Value>=rows.into_iter().map(|r| json!({"command_id":r.get::<Uuid,_>("command_id"),"room_id":r.get::<Uuid,_>("room_id"),"conversation_id":r.get::<Uuid,_>("conversation_id"),"type":r.get::<String,_>("command_type"),"revision":r.get::<i64,_>("revision"),"payload":r.get::<Value,_>("payload")})).collect();
    Ok(reply(StatusCode::OK, json!({"commands":commands})))
}

async fn bee_command_ack(
    State(access): State<Access>,
    axum::extract::Path(command): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    require_bee(&headers, &access)?;
    let id = Uuid::parse_str(&command).map_err(|_| invalid())?;
    let mut tx = access.pool.begin().await?;
    let row = sqlx::query("SELECT room_id,command_type,revision,delivered_at FROM bee_command_outbox WHERE command_id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?.ok_or_else(missing)?;
    if row
        .get::<Option<DateTime<Utc>>, _>("delivered_at")
        .is_some()
    {
        tx.commit().await?;
        return Ok(reply(
            StatusCode::OK,
            json!({"accepted":true,"duplicate":true}),
        ));
    }
    let room_id: Uuid = row.get("room_id");
    let ty: String = row.get("command_type");
    let rev: i64 = row.get("revision");
    sqlx::query("UPDATE bee_command_outbox SET delivered_at=now() WHERE command_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let changed = sqlx::query(
        "UPDATE bee_room_bindings SET status=$2,updated_at=now() WHERE room_id=$1 AND revision=$3",
    )
    .bind(room_id)
    .bind(if ty == "bind" { "bound" } else { "unbound" })
    .bind(rev)
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() > 0 && ty == "unbind" {
        sqlx::query("DELETE FROM bee_room_status WHERE room_id=$1")
            .bind(room_id)
            .execute(&mut *tx)
            .await?;
    }
    if changed.rows_affected() > 0 {
        change(&mut tx, room_id, false).await?;
    }
    tx.commit().await?;
    Ok(reply(StatusCode::OK, json!({"accepted":true})))
}

async fn bee_status(
    State(access): State<Access>,
    axum::extract::Path(room): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(status): Json<Value>,
) -> Result<Response> {
    require_bee(&headers, &access)?;
    let room = Uuid::parse_str(&room).map_err(|_| missing())?;
    if !status.is_object() || status.as_object().is_some_and(|o| o.len() > 16) {
        return Err(invalid());
    }
    let conversation = Uuid::parse_str(status["binding_id"].as_str().unwrap_or("")).ok();
    let mut tx = access.pool.begin().await?;
    let binding = sqlx::query(
        "SELECT conversation_id,status,revision FROM bee_room_bindings WHERE room_id=$1",
    )
    .bind(room)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(missing)?;
    let bound: Uuid = binding.get("conversation_id");
    let bind_status: String = binding.get("status");
    let revision: i64 = binding.get("revision");
    let incoming_revision = status["connectivity_revision"].as_i64();
    if incoming_revision.is_some_and(|incoming| incoming < revision) {
        return Err(conflict(
            "bee_binding_stale",
            "A newer binding revision superseded this status",
        ));
    }
    let valid_bind = status["status"] == "bound"
        && matches!(bind_status.as_str(), "binding" | "bound")
        && conversation == Some(bound);
    let valid_unbind =
        status["status"] == "unbound" && bind_status == "unbinding" && conversation.is_none();
    if incoming_revision != Some(revision) || !(valid_bind || valid_unbind) {
        return Err(conflict(
            "bee_binding_mismatch",
            "Status does not match room binding",
        ));
    }
    if let Some(previous) = sqlx::query("SELECT status FROM bee_room_status WHERE room_id=$1")
        .bind(room)
        .fetch_optional(&mut *tx)
        .await?
        && previous.get::<Value, _>("status") == status
    {
        tx.commit().await?;
        return Ok(reply(
            StatusCode::OK,
            json!({"accepted":true,"duplicate":true}),
        ));
    }
    sqlx::query("INSERT INTO bee_room_status(room_id,status) VALUES($1,$2) ON CONFLICT(room_id) DO UPDATE SET status=EXCLUDED.status,updated_at=now()").bind(room).bind(status).execute(&mut *tx).await?;
    change(&mut tx, room, false).await?;
    tx.commit().await?;
    Ok(reply(StatusCode::OK, json!({"accepted":true})))
}

async fn internal_ready(State(access): State<Access>, headers: HeaderMap) -> Result<Response> {
    let Some(credential) =
        h(&headers, "authorization").and_then(|value| value.strip_prefix("Bearer "))
    else {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "session_required",
            "Service authentication required",
        ));
    };
    if credential.is_empty() {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "session_required",
            "Service authentication required",
        ));
    }
    let bee = credential_matches(&access.security.bee_credential, credential);
    let topics = credential_matches(&access.security.topics_credential, credential);
    if !(bee | topics) {
        return Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "session_required",
            "Service authentication required",
        ));
    }
    let (status, body) = crate::health::ready(State(access.pool)).await;
    Ok((status, body).into_response())
}

fn credential_matches(configured: &Option<String>, presented: &str) -> bool {
    configured
        .as_ref()
        .is_some_and(|expected| equal(&hash(expected.as_bytes()), &hash(presented.as_bytes())))
}

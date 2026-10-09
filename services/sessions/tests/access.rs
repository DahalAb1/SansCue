use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sessions_service::{
    access::{Access, internal_router},
    app::configured_router,
    initialize, pool_options,
    security::{Security, hash},
};
use sqlx::{Executor, PgPool, Row, postgres::PgConnectOptions};
use std::str::FromStr;
use tower::ServiceExt;
use uuid::Uuid;
struct Browser {
    cookie: String,
    csrf: String,
}
struct Attempt<'a> {
    key: Uuid,
    origin: &'a str,
    csrf: Option<&'a str>,
}
fn attempt(key: Uuid) -> Attempt<'static> {
    Attempt {
        key,
        origin: "http://localhost:5173",
        csrf: None,
    }
}
async fn request(
    app: &Router,
    b: Option<&Browser>,
    method: &str,
    path: &str,
    body: Value,
    attempt: Attempt<'_>,
) -> (u16, Value, String) {
    let Attempt { key, origin, csrf } = attempt;
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("origin", origin)
        .header("content-type", "application/json")
        .header("idempotency-key", key.to_string());
    if let Some(b) = b {
        req = req
            .header("cookie", &b.cookie)
            .header("x-csrf-token", csrf.unwrap_or(&b.csrf));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let cookie = res
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, cookie)
}
async fn boot(app: &Router) -> Browser {
    let (status, v, c) = request(
        app,
        None,
        "GET",
        "/bootstrap",
        json!({}),
        attempt(Uuid::new_v4()),
    )
    .await;
    assert_eq!(status, 200);
    assert!(c.contains("HttpOnly") && c.contains("SameSite=Lax") && c.contains("Max-Age=86400"));
    assert!(!c.split(';').any(|part| part.trim() == "Secure"));
    Browser {
        cookie: c.split(';').next().unwrap().into(),
        csrf: v["csrf_token"].as_str().unwrap().into(),
    }
}
async fn call(app: &Router, b: &Browser, method: &str, path: &str, body: Value) -> (u16, Value) {
    let (s, v, _) = request(app, Some(b), method, path, body, attempt(Uuid::new_v4())).await;
    (s, v)
}
async fn db() -> (PgPool, PgPool, String) {
    let url=std::env::var("DATABASE_URL").expect("DATABASE_URL must point to a disposable PostgreSQL 17 database; access DB tests are never skipped");
    let admin = pool_options().connect(&url).await.unwrap();
    let schema = format!("access_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    (initialize(options).await.unwrap(), admin, schema)
}
fn security() -> Security {
    Security {
        origin: Some("http://localhost:5173".into()),
        insecure_loopback: true,
        operator_hash: Some(hash(b"synthetic-operator")),
        operator_epoch: Some("test-1".into()),
        hmac_key: Some(vec![7; 32]),
        bee_credential: Some("synthetic-bee".into()),
        ..Default::default()
    }
}
async fn login(app: &Router, b: &Browser) {
    assert_eq!(
        call(
            app,
            b,
            "POST",
            "/operator/login",
            json!({"credential":"synthetic-operator"})
        )
        .await
        .0,
        200
    );
}
#[tokio::test]
async fn access_contract_transactions_security_and_recovery() {
    let (pool, admin, schema) = db().await;
    let app = configured_router(pool.clone(), security());
    let host = boot(&app).await;
    let guest = boot(&app).await;
    let other = boot(&app).await;
    assert_eq!(
        call(&app, &host, "POST", "/rooms", json!({"title":"Talk"}))
            .await
            .0,
        403
    );
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            "/operator/login",
            json!({"credential":"wrong"})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            &app,
            Some(&host),
            "POST",
            "/operator/login",
            json!({"credential":"synthetic-operator"}),
            Attempt {
                origin: "https://evil.invalid",
                ..attempt(Uuid::new_v4())
            }
        )
        .await
        .1["code"],
        "origin_denied"
    );
    assert_eq!(
        request(
            &app,
            Some(&host),
            "POST",
            "/operator/login",
            json!({"credential":"synthetic-operator"}),
            Attempt {
                csrf: Some("wrong"),
                ..attempt(Uuid::new_v4())
            }
        )
        .await
        .1["code"],
        "csrf_failed"
    );
    login(&app, &host).await;
    let key = Uuid::new_v4();
    let created = request(
        &app,
        Some(&host),
        "POST",
        "/rooms",
        json!({"title":" Talk "}),
        attempt(key),
    )
    .await;
    assert_eq!(created.0, 201);
    assert_eq!(created.1["state"]["room"]["title"], "Talk");
    assert_eq!(created.1["state"]["membership"]["role"], "speaker");
    assert_eq!(created.1["state"]["join_link_enabled"], true);
    assert_eq!(
        created.1["state"]["revision"],
        created.1["state"]["sequence"]
    );
    assert_eq!(created.1["state"]["dashboard"]["respondents"], 0);
    assert!(created.1["state"]["dashboard"]["percentages"]["clear"].is_null());
    let rid = created.1["state"]["room"]["id"].as_str().unwrap();
    let root = format!("/rooms/{rid}");
    let join = created.1["join_url"].as_str().unwrap();
    let replay = request(
        &app,
        Some(&host),
        "POST",
        "/rooms",
        json!({"title":" Talk "}),
        attempt(key),
    )
    .await;
    assert_eq!(replay.0, 201);
    assert_eq!(
        replay.1["state"]["room"]["id"],
        created.1["state"]["room"]["id"]
    );
    assert!(replay.1["join_url"].is_null());
    assert_eq!(replay.1["link_unavailable"], true);
    assert_eq!(
        request(
            &app,
            Some(&host),
            "POST",
            "/rooms",
            json!({"title":"Other"}),
            attempt(key)
        )
        .await
        .1["code"],
        "idempotency_conflict"
    );
    assert_eq!(
        call(&app, &guest, "POST", join, json!({"role":"speaker"}))
            .await
            .0,
        400
    );
    assert_eq!(
        call(&app, &guest, "POST", "/join/invalid", json!({}))
            .await
            .0,
        404
    );
    let (s, g) = call(&app, &guest, "POST", join, json!({})).await;
    assert_eq!(s, 200);
    assert_eq!(g["state"]["membership"]["role"], "audience");
    assert!(g["state"]["dashboard"].is_null() && g["state"]["bee"].is_null());
    assert!(g["state"].get("join_link_enabled").is_none());
    let mid = g["state"]["membership"]["id"].as_str().unwrap();
    assert_eq!(
        call(&app, &guest, "POST", join, json!({})).await.1["state"]["membership"]["id"],
        mid
    );
    for route in ["memberships", "invitations"] {
        assert_eq!(
            call(&app, &guest, "GET", &format!("{root}/{route}"), json!({}))
                .await
                .0,
            403
        );
    }
    assert_eq!(
        call(&app, &other, "GET", &format!("{root}/state"), json!({}))
            .await
            .0,
        404
    );
    let (s, inv) = call(
        &app,
        &host,
        "POST",
        &format!("{root}/invitations"),
        json!({}),
    )
    .await;
    assert_eq!(s, 201);
    let redeem = format!(
        "/invitations/{}/redeem",
        inv["invite_url"]
            .as_str()
            .unwrap()
            .trim_start_matches("/invite/")
    );
    let ikey = Uuid::new_v4();
    assert_eq!(
        call(&app, &host, "POST", &redeem, json!({})).await.1["code"],
        "invite_used"
    );
    let ta = request(
        &app,
        Some(&guest),
        "POST",
        &redeem,
        json!({}),
        attempt(ikey),
    )
    .await;
    assert_eq!(ta.0, 200);
    assert!(ta.1["state"]["dashboard"].is_object());
    assert!(ta.1["state"].get("join_link_enabled").is_none());
    assert_eq!(ta.1["state"]["bee"]["status"], "unbound");
    assert_eq!(
        call(
            &app,
            &guest,
            "GET",
            &format!("{root}/memberships"),
            json!({})
        )
        .await
        .0,
        403
    );
    assert_eq!(ta.1["state"]["membership"]["role"], "ta");
    assert_eq!(
        request(
            &app,
            Some(&guest),
            "POST",
            &redeem,
            json!({}),
            attempt(ikey)
        )
        .await
        .1,
        ta.1
    );
    assert_eq!(
        call(&app, &other, "POST", &redeem, json!({})).await.1["code"],
        "invite_used"
    );
    assert_eq!(
        call(&app, &guest, "POST", &format!("{root}/end"), json!({}))
            .await
            .0,
        403
    );
    let revoke = format!(
        "{root}/invitations/{}/revoke",
        inv["invitation_id"].as_str().unwrap()
    );
    assert_eq!(call(&app, &host, "POST", &revoke, json!({})).await.0, 200);
    assert_eq!(
        call(&app, &guest, "GET", &format!("{root}/state"), json!({}))
            .await
            .0,
        404
    );
    assert_eq!(call(&app, &guest, "POST", join, json!({})).await.0, 404);
    let (_, inv2) = call(
        &app,
        &host,
        "POST",
        &format!("{root}/invitations"),
        json!({}),
    )
    .await;
    let redeem2 = format!(
        "/invitations/{}/redeem",
        inv2["invite_url"]
            .as_str()
            .unwrap()
            .trim_start_matches("/invite/")
    );
    assert_eq!(call(&app, &guest, "POST", &redeem2, json!({})).await.0, 200);
    call(&app, &host, "POST", &revoke, json!({})).await;
    assert_eq!(
        call(&app, &guest, "GET", &format!("{root}/state"), json!({}))
            .await
            .0,
        200
    );
    let (_, list) = call(
        &app,
        &host,
        "GET",
        &format!("{root}/memberships?limit=1"),
        json!({}),
    )
    .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert!(!list["next_cursor"].is_null());
    let (_, list2) = call(
        &app,
        &host,
        "GET",
        &format!(
            "{root}/memberships?after={}&limit=1",
            list["next_cursor"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    assert_ne!(
        list["items"][0]["membership_id"],
        list2["items"][0]["membership_id"]
    );
    let (_, expired) = call(
        &app,
        &host,
        "POST",
        &format!("{root}/invitations"),
        json!({}),
    )
    .await;
    sqlx::query("UPDATE invitations SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(Uuid::parse_str(expired["invitation_id"].as_str().unwrap()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            &other,
            "POST",
            &format!(
                "/invitations/{}/redeem",
                expired["invite_url"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("/invite/")
            ),
            json!({})
        )
        .await
        .0,
        404
    );
    let (_, rotated) = call(
        &app,
        &host,
        "POST",
        &format!("{root}/join-link/rotate"),
        json!({}),
    )
    .await;
    assert_eq!(call(&app, &other, "POST", join, json!({})).await.0, 404);
    assert_eq!(
        call(
            &app,
            &other,
            "POST",
            rotated["join_url"].as_str().unwrap(),
            json!({})
        )
        .await
        .0,
        200
    );
    let endkey = Uuid::new_v4();
    let endpath = format!("{root}/end");
    let (a, b) = tokio::join!(
        request(
            &app,
            Some(&host),
            "POST",
            &endpath,
            json!({}),
            attempt(endkey)
        ),
        request(
            &app,
            Some(&host),
            "POST",
            &endpath,
            json!({}),
            attempt(endkey)
        )
    );
    assert_eq!(a.0, 200);
    assert_eq!(a.1, b.1);
    assert_eq!(call(&app, &host, "POST", &endpath, json!({})).await.1, a.1);
    assert_eq!(
        call(&app, &guest, "GET", &format!("{root}/state"), json!({}))
            .await
            .1["room"]["status"],
        "ended"
    );
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            &format!("{root}/invitations"),
            json!({})
        )
        .await
        .1["code"],
        "room_ended"
    );
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            &format!("{root}/memberships/{mid}/revoke"),
            json!({})
        )
        .await
        .0,
        200
    );
    let mut rotated_security = security();
    rotated_security.operator_epoch = Some("test-2".into());
    let rotated_app = configured_router(pool.clone(), rotated_security);
    assert_eq!(
        call(
            &rotated_app,
            &host,
            "GET",
            &format!("{root}/state"),
            json!({})
        )
        .await
        .0,
        404
    );
    let restored = boot(&rotated_app).await;
    login(&rotated_app, &restored).await;
    assert_eq!(
        call(
            &rotated_app,
            &restored,
            "GET",
            &format!("{root}/state"),
            json!({})
        )
        .await
        .1["membership"]["id"],
        created.1["state"]["membership"]["id"]
    );
    let receipt_dump: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(body::text || target,''),'') FROM idempotency",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!receipt_dump.contains("synthetic-operator"));
    assert!(!receipt_dump.contains(join));
    assert!(!receipt_dump.contains("/join/"));
    assert!(!receipt_dump.contains("/invite/"));
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    admin.close().await;
}

async fn finish(pool: PgPool, admin: PgPool, schema: String) {
    pool.close().await;
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    admin.close().await;
}

async fn once(
    app: &Router,
    method: &str,
    path: &str,
    header: Option<(&str, &str)>,
) -> (u16, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some((name, value)) = header {
        builder = builder.header(name, value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn private_json(app: &Router, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", "Bearer synthetic-bee")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn private_topics_json(app: &Router, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", "Bearer synthetic-topics")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn topic_candidate(
    event: Uuid,
    conversation: Uuid,
    room: Uuid,
    session: Uuid,
    excerpt: &str,
) -> Value {
    json!({
        "candidate_id":event,
        "event_id":event,
        "room_id":room,
        "session_id":session,
        "text":format!("What is the implication of {excerpt}?"),
        "generator_version":"stub-v1",
        "evidence":{
            "conversation_id":conversation,
            "ingest_ordinal":1,
            "received_at":"2026-10-09T00:00:00Z",
            "excerpt":excerpt
        },
        "published":false
    })
}

fn token_of(browser: &Browser) -> &str {
    browser.cookie.split_once('=').unwrap().1
}

#[tokio::test]
async fn validation_expiry_lockout_and_cookie_flags() {
    let (pool, admin, schema) = db().await;
    let app = configured_router(pool.clone(), security());
    let (status, body, _) = request(
        &app,
        None,
        "GET",
        "/healthz",
        json!({}),
        attempt(Uuid::new_v4()),
    )
    .await;
    assert_eq!((status, body), (200, json!({"status":"ok"})));
    let (status, missing_route) = once(
        &app,
        "GET",
        "/internal/v1/readyz",
        Some(("authorization", "Bearer synthetic-bee")),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(missing_route["code"], "not_found");
    let host = boot(&app).await;
    assert_eq!(
        request(
            &app,
            None,
            "POST",
            "/rooms",
            json!({"title":"Talk"}),
            attempt(Uuid::new_v4())
        )
        .await
        .0,
        401
    );
    let bad_key = Request::builder()
        .method("POST")
        .uri("/rooms")
        .header("origin", "http://localhost:5173")
        .header("content-type", "application/json")
        .header("idempotency-key", "not-a-uuid")
        .header("cookie", &host.cookie)
        .header("x-csrf-token", &host.csrf)
        .body(Body::from(json!({"title":"Talk"}).to_string()))
        .unwrap();
    let bad = app.clone().oneshot(bad_key).await.unwrap();
    assert_eq!(bad.status().as_u16(), 400);
    login(&app, &host).await;
    let key = Uuid::new_v4();
    assert_eq!(
        request(
            &app,
            Some(&host),
            "POST",
            "/rooms",
            json!({"title":"   "}),
            attempt(key)
        )
        .await
        .0,
        400
    );
    assert_eq!(
        request(
            &app,
            Some(&host),
            "POST",
            "/rooms",
            json!({"title":"a".repeat(201)}),
            attempt(key)
        )
        .await
        .0,
        400
    );
    assert_eq!(
        request(
            &app,
            Some(&host),
            "POST",
            "/rooms",
            json!({"title":"Kept"}),
            attempt(key)
        )
        .await
        .0,
        201
    );
    let plain = Request::builder()
        .method("POST")
        .uri("/rooms")
        .header("origin", "http://localhost:5173")
        .header("content-type", "text/plain")
        .header("idempotency-key", Uuid::new_v4().to_string())
        .header("cookie", &host.cookie)
        .header("x-csrf-token", &host.csrf)
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(plain).await.unwrap().status().as_u16(),
        400
    );
    for _ in 0..5 {
        assert_eq!(
            call(
                &app,
                &host,
                "POST",
                "/operator/login",
                json!({"credential":"wrong"})
            )
            .await
            .0,
            403
        );
    }
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            "/operator/login",
            json!({"credential":"synthetic-operator"})
        )
        .await
        .1["code"],
        "rate_limited"
    );
    sqlx::query("UPDATE browser_sessions SET failure_window=now()-interval '16 minutes' WHERE token_hash=$1")
        .bind(hash(token_of(&host).as_bytes()))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            "/operator/login",
            json!({"credential":"wrong"})
        )
        .await
        .0,
        403
    );
    sqlx::query(
        "UPDATE browser_sessions SET last_seen_at=now()-interval '13 hours' WHERE token_hash=$1",
    )
    .bind(hash(token_of(&host).as_bytes()))
    .execute(&pool)
    .await
    .unwrap();
    let expired = boot_with(&app, Some(&host), "http://localhost:5173").await;
    assert_ne!(expired.csrf, host.csrf);
    assert_eq!(
        call(&app, &host, "POST", "/rooms", json!({"title":"Late"}))
            .await
            .0,
        401
    );
    sqlx::query(
        "UPDATE browser_sessions SET created_at=now()-interval '25 hours', last_seen_at=now() WHERE token_hash=$1",
    )
    .bind(hash(token_of(&expired).as_bytes()))
    .execute(&pool)
    .await
    .unwrap();
    let replaced = boot_with(&app, Some(&expired), "http://localhost:5173").await;
    assert_ne!(replaced.csrf, expired.csrf);
    let mut https = security();
    https.origin = Some("https://rooms.example".into());
    https.insecure_loopback = false;
    let secure_app = configured_router(pool.clone(), https);
    let (status, _, cookie) = request(
        &secure_app,
        None,
        "GET",
        "/bootstrap",
        json!({}),
        Attempt {
            origin: "https://rooms.example",
            ..attempt(Uuid::new_v4())
        },
    )
    .await;
    assert_eq!(status, 200);
    assert!(cookie.split(';').any(|part| part.trim() == "Secure"));
    let mut disabled = security();
    disabled.operator_hash = None;
    let disabled_app = configured_router(pool.clone(), disabled);
    let browser = boot(&disabled_app).await;
    assert_eq!(
        call(
            &disabled_app,
            &browser,
            "POST",
            "/operator/login",
            json!({"credential":"synthetic-operator"})
        )
        .await
        .0,
        403
    );
    finish(pool, admin, schema).await;
}

async fn boot_with(app: &Router, browser: Option<&Browser>, origin: &str) -> Browser {
    let (status, value, cookie) = request(
        app,
        browser,
        "GET",
        "/bootstrap",
        json!({}),
        Attempt {
            origin,
            ..attempt(Uuid::new_v4())
        },
    )
    .await;
    assert_eq!(status, 200);
    Browser {
        cookie: cookie.split(';').next().unwrap().into(),
        csrf: value["csrf_token"].as_str().unwrap().into(),
    }
}

#[tokio::test]
async fn revocation_replay_grants_and_role_boundaries() {
    let (pool, admin, schema) = db().await;
    let app = configured_router(pool.clone(), security());
    let host = boot(&app).await;
    login(&app, &host).await;
    let created = call(&app, &host, "POST", "/rooms", json!({"title":"Bounds"}))
        .await
        .1;
    let room = created["state"]["room"]["id"].as_str().unwrap();
    let root = format!("/rooms/{room}");
    let speaker = created["state"]["membership"]["id"].as_str().unwrap();
    let join = created["join_url"].as_str().unwrap().to_owned();
    let guest = boot(&app).await;
    let join_key = Uuid::new_v4();
    let joined = request(
        &app,
        Some(&guest),
        "POST",
        &join,
        json!({}),
        attempt(join_key),
    )
    .await;
    assert_eq!(joined.0, 200);
    let audience = joined.1["state"]["membership"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        call(&app, &guest, "GET", &format!("{root}/state"), json!({}))
            .await
            .1["membership"]["role"],
        "audience"
    );
    assert_eq!(
        request(
            &app,
            Some(&guest),
            "GET",
            &format!("{root}/state"),
            json!({}),
            Attempt {
                origin: "https://evil.invalid",
                ..attempt(Uuid::new_v4())
            }
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&app, &guest, "POST", &format!("{root}/end"), json!({}))
            .await
            .0,
        403
    );
    assert_eq!(
        call(
            &app,
            &guest,
            "POST",
            &format!("{root}/join-link/revoke"),
            json!({})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            &format!("{root}/memberships/{speaker}/revoke"),
            json!({})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            &format!("{root}/join-link/revoke"),
            json!({})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&app, &host, "GET", &format!("{root}/state"), json!({}))
            .await
            .1["join_link_enabled"],
        false
    );
    assert_eq!(
        call(&app, &guest, "GET", &format!("{root}/state"), json!({}))
            .await
            .0,
        200
    );
    let late = boot(&app).await;
    assert_eq!(call(&app, &late, "POST", &join, json!({})).await.0, 404);
    let rotated = call(
        &app,
        &host,
        "POST",
        &format!("{root}/join-link/rotate"),
        json!({}),
    )
    .await
    .1;
    assert_eq!(call(&app, &late, "POST", &join, json!({})).await.0, 404);
    let replay = request(
        &app,
        Some(&guest),
        "POST",
        &join,
        json!({}),
        attempt(join_key),
    )
    .await;
    assert_eq!(replay.0, 200);
    assert_eq!(replay.1["state"]["membership"]["id"], audience);
    let late_join = call(
        &app,
        &late,
        "POST",
        rotated["join_url"].as_str().unwrap(),
        json!({}),
    )
    .await;
    assert_eq!(late_join.0, 200);
    let late_id = late_join.1["state"]["membership"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let audience_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM memberships WHERE room_id=$1 AND role='audience'")
            .bind(Uuid::parse_str(room).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audience_count, 2);
    login(&app, &guest).await;
    assert_eq!(
        call(&app, &guest, "GET", &format!("{root}/state"), json!({}))
            .await
            .1["membership"]["id"],
        speaker
    );
    let speakers: i64 =
        sqlx::query_scalar("SELECT count(*) FROM memberships WHERE room_id=$1 AND role='speaker'")
            .bind(Uuid::parse_str(room).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(speakers, 1);
    let listed = call(
        &app,
        &host,
        "GET",
        &format!("{root}/memberships?limit=1"),
        json!({}),
    )
    .await
    .1;
    let mut seen = Vec::new();
    let mut page = listed;
    loop {
        let items = page["items"].as_array().unwrap();
        assert!(items.len() <= 1);
        for item in items {
            assert!(item.get("session_id").is_none());
            assert!(item["current_grant_id"].is_null());
            assert!(!item["label"].as_str().unwrap().contains('@'));
            seen.push(item["membership_id"].as_str().unwrap().to_owned());
        }
        let Some(cursor) = page["next_cursor"].as_str() else {
            break;
        };
        page = call(
            &app,
            &host,
            "GET",
            &format!("{root}/memberships?after={cursor}&limit=1"),
            json!({}),
        )
        .await
        .1;
    }
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3
    );
    let (status, invite) = call(
        &app,
        &host,
        "POST",
        &format!("{root}/invitations"),
        json!({}),
    )
    .await;
    assert_eq!(status, 201);
    let invite_id = invite["invitation_id"].as_str().unwrap();
    sqlx::query("UPDATE invitations SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(Uuid::parse_str(invite_id).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let invitations = call(
        &app,
        &host,
        "GET",
        &format!("{root}/invitations"),
        json!({}),
    )
    .await
    .1;
    let expired = invitations["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["invitation_id"].as_str() == Some(invite_id))
        .unwrap();
    assert_eq!(expired["status"], "expired");
    assert!(expired.get("invite_url").is_none());
    let other_room = call(&app, &host, "POST", "/rooms", json!({"title":"Other"}))
        .await
        .1;
    let other = other_room["state"]["room"]["id"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            &format!("/rooms/{other}/memberships/{audience}/revoke"),
            json!({})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(&app, &late, "GET", &format!("{root}/state"), json!({}))
            .await
            .0,
        200
    );
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            &format!("{root}/memberships/{late_id}/revoke"),
            json!({})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&app, &late, "GET", &format!("{root}/state"), json!({}))
            .await
            .0,
        404
    );
    assert_eq!(
        call(
            &app,
            &late,
            "POST",
            rotated["join_url"].as_str().unwrap(),
            json!({})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        request(
            &app,
            Some(&guest),
            "POST",
            &join,
            json!({}),
            attempt(join_key)
        )
        .await
        .1["state"]["membership"]["id"],
        audience
    );
    finish(pool, admin, schema).await;
}

#[tokio::test]
async fn ended_room_keeps_reads_and_replays_without_new_joins() {
    let (pool, admin, schema) = db().await;
    let app = configured_router(pool.clone(), security());
    let host = boot(&app).await;
    login(&app, &host).await;
    let created = call(&app, &host, "POST", "/rooms", json!({"title":"Ending"}))
        .await
        .1;
    let room = created["state"]["room"]["id"].as_str().unwrap();
    let root = format!("/rooms/{room}");
    let join = created["join_url"].as_str().unwrap().to_owned();
    let guest = boot(&app).await;
    let join_key = Uuid::new_v4();
    let joined = request(
        &app,
        Some(&guest),
        "POST",
        &join,
        json!({}),
        attempt(join_key),
    )
    .await;
    let membership = joined.1["state"]["membership"]["id"].clone();
    let (status, invite) = call(
        &app,
        &host,
        "POST",
        &format!("{root}/invitations"),
        json!({}),
    )
    .await;
    assert_eq!(status, 201);
    let redeem = format!(
        "/invitations/{}/redeem",
        invite["invite_url"]
            .as_str()
            .unwrap()
            .trim_start_matches("/invite/")
    );
    let ended = call(&app, &host, "POST", &format!("{root}/end"), json!({}))
        .await
        .1;
    assert_eq!(ended["status"], "ended");
    assert_eq!(call(&app, &guest, "POST", &join, json!({})).await.0, 404);
    assert_eq!(call(&app, &guest, "POST", &redeem, json!({})).await.0, 404);
    let replay = request(
        &app,
        Some(&guest),
        "POST",
        &join,
        json!({}),
        attempt(join_key),
    )
    .await;
    assert_eq!(replay.0, 200);
    assert_eq!(replay.1["state"]["membership"]["id"], membership);
    assert_eq!(
        call(&app, &guest, "GET", &format!("{root}/state"), json!({}))
            .await
            .1["room"]["status"],
        "ended"
    );
    assert_eq!(
        call(
            &app,
            &host,
            "POST",
            &format!("{root}/invitations"),
            json!({})
        )
        .await
        .1["code"],
        "room_ended"
    );
    let repeated = call(&app, &host, "POST", &format!("{root}/end"), json!({}))
        .await
        .1;
    assert_eq!(repeated["revision"], ended["revision"]);
    assert_eq!(
        repeated["revision"],
        replay_revision(&app, &host, &root).await
    );
    finish(pool, admin, schema).await;
}

async fn replay_revision(app: &Router, host: &Browser, root: &str) -> Value {
    call(app, host, "GET", &format!("{root}/state"), json!({}))
        .await
        .1["revision"]
        .clone()
}

#[tokio::test]
async fn private_readiness_is_not_a_public_route() {
    let (pool, admin, schema) = db().await;
    let app = configured_router(pool.clone(), security());
    let private = internal_router(Access {
        pool: pool.clone(),
        security: security(),
    });
    assert_eq!(
        once(&private, "GET", "/internal/v1/readyz", None).await.0,
        401
    );
    assert_eq!(
        once(
            &private,
            "GET",
            "/internal/v1/readyz",
            Some(("authorization", "Bearer no-such"))
        )
        .await
        .0,
        401
    );
    assert_eq!(
        once(
            &private,
            "GET",
            "/healthz",
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        404
    );
    let (status, body) = once(
        &private,
        "GET",
        "/internal/v1/readyz",
        Some(("authorization", "Bearer synthetic-bee")),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({"status":"ready"}));
    let (status, body) = once(&app, "GET", "/readyz", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ready");
    finish(pool, admin, schema).await;
}

#[tokio::test]
async fn bee_binding_is_speaker_authoritative_and_private_commands_are_retryable() {
    let (pool, admin, schema) = db().await;
    let security = Security {
        bee_credential: Some("synthetic-bee".into()),
        ..security()
    };
    let app = configured_router(pool.clone(), security.clone());
    let private = internal_router(Access {
        pool: pool.clone(),
        security,
    });
    let browser = boot(&app).await;
    login(&app, &browser).await;
    let (_, created) = call(
        &app,
        &browser,
        "POST",
        "/rooms",
        json!({"title":"Bee test"}),
    )
    .await;
    let room = created["state"]["room"]["id"].as_str().unwrap();
    let conversation = Uuid::new_v4();
    let (status, bound) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/bind"),
        json!({"conversation_id":conversation,"source_conversation_id":null}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(bound["status"], "binding");
    let state = call(
        &app,
        &browser,
        "GET",
        &format!("/rooms/{room}/state"),
        json!({}),
    )
    .await
    .1;
    assert_eq!(state["bee"]["status"], "binding");
    let mut request = Request::builder()
        .uri("/internal/v1/bee/commands")
        .header("authorization", "Bearer synthetic-bee")
        .body(Body::empty())
        .unwrap();
    let commands = private.clone().oneshot(request).await.unwrap();
    assert_eq!(commands.status(), 200);
    let bytes = commands.into_body().collect().await.unwrap().to_bytes();
    let list: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list["commands"].as_array().unwrap().len(), 1);
    assert!(
        Uuid::parse_str(
            list["commands"][0]["payload"]["session_id"]
                .as_str()
                .unwrap()
        )
        .is_ok()
    );
    let command = list["commands"][0]["command_id"].as_str().unwrap();
    let revision = bound["revision"].as_i64().unwrap();
    assert_eq!(
        call(
            &app,
            &browser,
            "GET",
            &format!("/rooms/{room}/state"),
            json!({})
        )
        .await
        .1["bee"]["status"],
        "binding",
        "status publication alone is not command acknowledgement"
    );
    let status_body = json!({"binding_id":conversation,"status":"bound","has_gaps":false,"connectivity_revision":revision});
    assert_eq!(
        private_json(
            &private,
            "POST",
            &format!("/internal/v1/bee/rooms/{room}/status"),
            status_body.clone()
        )
        .await
        .0,
        200
    );
    request = Request::builder()
        .method("POST")
        .uri(format!("/internal/v1/bee/commands/{command}/ack"))
        .header("authorization", "Bearer synthetic-bee")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        private.clone().oneshot(request).await.unwrap().status(),
        200
    );
    assert_eq!(
        call(
            &app,
            &browser,
            "GET",
            &format!("/rooms/{room}/state"),
            json!({})
        )
        .await
        .1["bee"]["status"],
        "bound"
    );
    let before: i64 = sqlx::query_scalar("SELECT sequence FROM rooms WHERE id=$1")
        .bind(Uuid::parse_str(room).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    let duplicate = once(
        &private,
        "POST",
        &format!("/internal/v1/bee/commands/{command}/ack"),
        Some(("authorization", "Bearer synthetic-bee")),
    )
    .await;
    assert_eq!(duplicate.0, 200);
    let after: i64 = sqlx::query_scalar("SELECT sequence FROM rooms WHERE id=$1")
        .bind(Uuid::parse_str(room).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after, "duplicate ack must not emit a room event");

    let (_, unbind) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/unbind"),
        json!({}),
    )
    .await;
    let unbind_command = unbind["command_id"].as_str().unwrap();
    assert_eq!(
        once(
            &private,
            "POST",
            &format!("/internal/v1/bee/commands/{unbind_command}/ack"),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    let (_, new_binding) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/bind"),
        json!({"conversation_id":Uuid::new_v4(),"source_conversation_id":null}),
    )
    .await;
    let (_, commands) = once(
        &private,
        "GET",
        "/internal/v1/bee/commands",
        Some(("authorization", "Bearer synthetic-bee")),
    )
    .await;
    let new_command = commands["commands"][0]["command_id"].as_str().unwrap();
    let new_conversation = Uuid::parse_str(
        commands["commands"][0]["payload"]["conversation_id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(private_json(&private,"POST",&format!("/internal/v1/bee/rooms/{room}/status"),json!({"binding_id":new_conversation,"status":"bound","has_gaps":false,"connectivity_revision":new_binding["revision"]})).await.0,200);
    assert_eq!(
        once(
            &private,
            "POST",
            &format!("/internal/v1/bee/commands/{new_command}/ack"),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    let events_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM room_events WHERE room_id=$1")
            .bind(Uuid::parse_str(room).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        once(
            &private,
            "POST",
            &format!("/internal/v1/bee/commands/{unbind_command}/ack"),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM room_events WHERE room_id=$1")
        .bind(Uuid::parse_str(room).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        events_before, events_after,
        "old duplicate unbind ack must not emit a room event"
    );
    let current = call(
        &app,
        &browser,
        "GET",
        &format!("/rooms/{room}/state"),
        json!({}),
    )
    .await
    .1;
    assert_eq!(current["bee"]["binding_id"], new_conversation.to_string());
    assert_eq!(current["bee"]["status"], "bound");
    finish(pool, admin, schema).await;
}

#[tokio::test]
async fn superseded_binding_status_has_machine_readable_stale_outcome() {
    let (pool, admin, schema) = db().await;
    let security = Security {
        bee_credential: Some("synthetic-bee".into()),
        ..security()
    };
    let app = configured_router(pool.clone(), security.clone());
    let private = internal_router(Access {
        pool: pool.clone(),
        security,
    });
    let browser = boot(&app).await;
    login(&app, &browser).await;
    let (_, created) = call(
        &app,
        &browser,
        "POST",
        "/rooms",
        json!({"title":"stale status"}),
    )
    .await;
    let room = created["state"]["room"]["id"].as_str().unwrap();
    let conversation = Uuid::new_v4();
    let (_, bind) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/bind"),
        json!({"conversation_id":conversation,"source_conversation_id":null}),
    )
    .await;
    let (_, unbind) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/unbind"),
        json!({}),
    )
    .await;
    let stale=private_json(&private,"POST",&format!("/internal/v1/bee/rooms/{room}/status"),json!({"binding_id":conversation,"status":"bound","connectivity_revision":bind["revision"]})).await;
    assert_eq!(stale.0, 409);
    assert_eq!(stale.1["code"], "bee_binding_stale");
    assert_eq!(
        private_json(
            &private,
            "POST",
            &format!("/internal/v1/bee/rooms/{room}/status"),
            json!({"binding_id":null,"status":"unbound","connectivity_revision":unbind["revision"]})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        once(
            &private,
            "POST",
            &format!(
                "/internal/v1/bee/commands/{}/ack",
                bind["command_id"].as_str().unwrap()
            ),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        once(
            &private,
            "POST",
            &format!(
                "/internal/v1/bee/commands/{}/ack",
                unbind["command_id"].as_str().unwrap()
            ),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    let current = call(
        &app,
        &browser,
        "GET",
        &format!("/rooms/{room}/state"),
        json!({}),
    )
    .await
    .1;
    assert_eq!(current["bee"]["status"], "unbound");
    finish(pool, admin, schema).await;
}

#[tokio::test]
async fn topic_candidate_receipts_survive_unbind_and_room_end() {
    let (pool, admin, schema) = db().await;
    let sec = Security {
        topics_credential: Some("synthetic-topics".into()),
        ..security()
    };
    let app = configured_router(pool.clone(), sec.clone());
    let private = internal_router(Access {
        pool: pool.clone(),
        security: sec,
    });
    let browser = boot(&app).await;
    login(&app, &browser).await;
    let (_, created) = call(
        &app,
        &browser,
        "POST",
        "/rooms",
        json!({"title":"candidate receipt"}),
    )
    .await;
    let room = created["state"]["room"]["id"].as_str().unwrap().to_owned();
    let conversation = Uuid::new_v4();
    let (_, binding) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/bind"),
        json!({"conversation_id":conversation,"source_conversation_id":null}),
    )
    .await;
    let session =
        sqlx::query_scalar::<_, Uuid>("SELECT session_id FROM bee_room_bindings WHERE room_id=$1")
            .bind(Uuid::parse_str(&room).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(private_json(&private,"POST",&format!("/internal/v1/bee/rooms/{room}/status"),json!({"binding_id":conversation,"status":"bound","has_gaps":false,"connectivity_revision":binding["revision"]})).await.0,200);
    assert_eq!(
        once(
            &private,
            "POST",
            &format!(
                "/internal/v1/bee/commands/{}/ack",
                binding["command_id"].as_str().unwrap()
            ),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );

    let candidate_id = Uuid::new_v4();
    let candidate = topic_candidate(
        candidate_id,
        conversation,
        Uuid::parse_str(&room).unwrap(),
        session,
        "stable evidence",
    );
    assert_eq!(
        private_topics_json(
            &private,
            "POST",
            "/internal/v1/topics/candidates",
            candidate.clone()
        )
        .await
        .0,
        202
    );
    let (_, unbind) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/unbind"),
        json!({}),
    )
    .await;
    assert_eq!(private_json(&private,"POST",&format!("/internal/v1/bee/rooms/{room}/status"),json!({"binding_id":null,"status":"unbound","has_gaps":false,"connectivity_revision":unbind["revision"]})).await.0,200);
    assert_eq!(
        once(
            &private,
            "POST",
            &format!(
                "/internal/v1/bee/commands/{}/ack",
                unbind["command_id"].as_str().unwrap()
            ),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    let duplicate = private_topics_json(
        &private,
        "POST",
        "/internal/v1/topics/candidates",
        candidate.clone(),
    )
    .await;
    assert_eq!(duplicate.0, 200);
    assert_eq!(duplicate.1["duplicate"], true);
    let mut conflict = candidate.clone();
    conflict["text"] = json!("different text for same ID");
    let conflicting =
        private_topics_json(&private, "POST", "/internal/v1/topics/candidates", conflict).await;
    assert_eq!(conflicting.0, 409);
    assert_eq!(conflicting.1["code"], "topic_candidate_conflict");
    let unseen = topic_candidate(
        Uuid::new_v4(),
        conversation,
        Uuid::parse_str(&room).unwrap(),
        session,
        "never accepted",
    );
    let stale =
        private_topics_json(&private, "POST", "/internal/v1/topics/candidates", unseen).await;
    assert_eq!(stale.0, 410);
    assert_eq!(stale.1["code"], "topic_candidate_stale");

    let conversation2 = Uuid::new_v4();
    let (_, binding2) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/bee/bind"),
        json!({"conversation_id":conversation2,"source_conversation_id":null}),
    )
    .await;
    assert_eq!(private_json(&private,"POST",&format!("/internal/v1/bee/rooms/{room}/status"),json!({"binding_id":conversation2,"status":"bound","has_gaps":false,"connectivity_revision":binding2["revision"]})).await.0,200);
    assert_eq!(
        once(
            &private,
            "POST",
            &format!(
                "/internal/v1/bee/commands/{}/ack",
                binding2["command_id"].as_str().unwrap()
            ),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    let (_, ended) = call(
        &app,
        &browser,
        "POST",
        &format!("/rooms/{room}/end"),
        json!({}),
    )
    .await;
    assert_eq!(ended["status"], "ended");
    let replay_after_end = private_topics_json(
        &private,
        "POST",
        "/internal/v1/topics/candidates",
        candidate,
    )
    .await;
    assert_eq!(replay_after_end.0, 200);
    let stale_after_end = private_topics_json(
        &private,
        "POST",
        "/internal/v1/topics/candidates",
        topic_candidate(
            Uuid::new_v4(),
            conversation2,
            Uuid::parse_str(&room).unwrap(),
            session,
            "never accepted after end",
        ),
    )
    .await;
    assert_eq!(stale_after_end.0, 410);
    assert_eq!(stale_after_end.1["code"], "topic_candidate_stale");
    finish(pool, admin, schema).await;
}

#[tokio::test]
async fn published_questions_feedback_and_written_questions_are_role_partitioned() {
    let (pool, admin, schema) = db().await;
    let sec = Security {
        topics_credential: Some("synthetic-topics".into()),
        ..security()
    };
    let app = configured_router(pool.clone(), sec.clone());
    let private = internal_router(Access {
        pool: pool.clone(),
        security: sec,
    });
    let speaker = boot(&app).await;
    let audience = boot(&app).await;
    let other_audience = boot(&app).await;
    login(&app, &speaker).await;
    let (_, created) = call(
        &app,
        &speaker,
        "POST",
        "/rooms",
        json!({"title":"Publication"}),
    )
    .await;
    let room_text = created["state"]["room"]["id"].as_str().unwrap();
    let room_id = Uuid::parse_str(room_text).unwrap();
    let root = format!("/rooms/{room_text}");
    let join = created["join_url"].as_str().unwrap();
    assert_eq!(call(&app, &audience, "POST", join, json!({})).await.0, 200);
    assert_eq!(
        call(&app, &other_audience, "POST", join, json!({})).await.0,
        200
    );

    let conversation = Uuid::new_v4();
    let (_, binding) = call(
        &app,
        &speaker,
        "POST",
        &format!("{root}/bee/bind"),
        json!({"conversation_id":conversation,"source_conversation_id":null}),
    )
    .await;
    let session =
        sqlx::query_scalar::<_, Uuid>("SELECT session_id FROM bee_room_bindings WHERE room_id=$1")
            .bind(room_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(private_json(&private,"POST",&format!("/internal/v1/bee/rooms/{room_text}/status"),json!({"binding_id":conversation,"status":"bound","has_gaps":false,"connectivity_revision":binding["revision"]})).await.0,200);
    assert_eq!(
        once(
            &private,
            "POST",
            &format!(
                "/internal/v1/bee/commands/{}/ack",
                binding["command_id"].as_str().unwrap()
            ),
            Some(("authorization", "Bearer synthetic-bee"))
        )
        .await
        .0,
        200
    );
    let candidate_id = Uuid::new_v4();
    let candidate = topic_candidate(
        candidate_id,
        conversation,
        room_id,
        session,
        "a private source excerpt",
    );
    assert_eq!(
        private_topics_json(
            &private,
            "POST",
            "/internal/v1/topics/candidates",
            candidate.clone()
        )
        .await
        .0,
        202
    );
    assert_eq!(
        call(
            &app,
            &audience,
            "POST",
            &format!("{root}/questions/{candidate_id}/publish"),
            json!({})
        )
        .await
        .0,
        403
    );

    let (_, published) = call(
        &app,
        &speaker,
        "POST",
        &format!("{root}/questions/{candidate_id}/publish"),
        json!({}),
    )
    .await;
    assert_eq!(published["published"], true);
    let question_id = published["question_id"].as_str().unwrap();
    let (status, audience_state) =
        call(&app, &audience, "GET", &format!("{root}/state"), json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(audience_state["active_question"]["text"], candidate["text"]);
    assert!(audience_state["active_question"].get("evidence").is_none());
    assert!(audience_state["question_candidates"].is_null());
    assert_eq!(audience_state["my_response"], Value::Null);
    assert_eq!(
        call(
            &app,
            &speaker,
            "POST",
            &format!("{root}/questions/{question_id}/response"),
            json!({"response":"clear"})
        )
        .await
        .0,
        403
    );
    let response_path = format!("{root}/questions/{question_id}/response");
    let response_key = Uuid::new_v4();
    let response_body = json!({"response":"clear"});
    assert_eq!(
        request(
            &app,
            Some(&audience),
            "POST",
            &response_path,
            response_body.clone(),
            attempt(response_key)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &app,
            Some(&audience),
            "POST",
            &response_path,
            response_body,
            attempt(response_key)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            &audience,
            "POST",
            &format!("{root}/questions/{question_id}/response"),
            json!({"response":"need_help"})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            &other_audience,
            "POST",
            &format!("{root}/questions/{question_id}/response"),
            json!({"response":"partly_clear"})
        )
        .await
        .0,
        200
    );
    let (_, speaker_state) = call(&app, &speaker, "GET", &format!("{root}/state"), json!({})).await;
    assert_eq!(speaker_state["dashboard"]["respondents"], 2);
    assert_eq!(speaker_state["dashboard"]["counts"]["need_help"], 1);
    assert_eq!(speaker_state["dashboard"]["counts"]["partly_clear"], 1);
    assert!(
        speaker_state["question_candidates"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (_, audience_state) =
        call(&app, &audience, "GET", &format!("{root}/state"), json!({})).await;
    assert_eq!(audience_state["my_response"], "need_help");
    assert!(audience_state.get("dashboard").is_none_or(Value::is_null));

    assert_eq!(
        call(
            &app,
            &speaker,
            "POST",
            &format!("{root}/questions/{question_id}/versions"),
            json!({"text":"Revised for clarity"})
        )
        .await
        .0,
        200
    );
    let evidence_rows = sqlx::query("SELECT version,evidence,question_text FROM published_question_versions WHERE question_id=$1 ORDER BY version")
        .bind(Uuid::parse_str(question_id).unwrap()).fetch_all(&pool).await.unwrap();
    assert_eq!(evidence_rows.len(), 2);
    assert_eq!(
        evidence_rows[0].get::<Value, _>("evidence"),
        evidence_rows[1].get::<Value, _>("evidence")
    );
    assert_ne!(
        evidence_rows[0].get::<String, _>("question_text"),
        evidence_rows[1].get::<String, _>("question_text")
    );
    assert!(sqlx::query("UPDATE published_question_versions SET question_text='mutated' WHERE question_id=$1 AND version=1")
        .bind(Uuid::parse_str(question_id).unwrap()).execute(&pool).await.is_err());
    let (_, public_state) = call(&app, &audience, "GET", &format!("{root}/state"), json!({})).await;
    assert_eq!(public_state["active_question"]["version"], 2);
    assert_eq!(
        public_state["active_question"]["text"],
        "Revised for clarity"
    );

    let private_text = "A confidential written question";
    let qa_path = format!("{root}/qa");
    let qa_key = Uuid::new_v4();
    let qa_body = json!({"body":private_text,"question_id":question_id});
    assert_eq!(
        request(
            &app,
            Some(&audience),
            "POST",
            &qa_path,
            qa_body.clone(),
            attempt(qa_key)
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &app,
            Some(&audience),
            "POST",
            &qa_path,
            qa_body,
            attempt(qa_key)
        )
        .await
        .0,
        200
    );
    let (_, own_qa) = call(&app, &audience, "GET", &format!("{root}/qa"), json!({})).await;
    assert_eq!(own_qa["items"].as_array().unwrap().len(), 1);
    assert_eq!(own_qa["items"][0]["question_id"], question_id);
    let (_, other_qa) = call(
        &app,
        &other_audience,
        "GET",
        &format!("{root}/qa"),
        json!({}),
    )
    .await;
    assert_eq!(other_qa["items"].as_array().unwrap().len(), 0);
    let (_, staff_qa) = call(&app, &speaker, "GET", &format!("{root}/qa"), json!({})).await;
    assert_eq!(
        staff_qa["items"].as_array().unwrap()[0]["body"],
        private_text
    );
    assert_eq!(
        call(
            &app,
            &audience,
            "POST",
            &format!("{root}/qa"),
            json!({"body":"x".repeat(4001)})
        )
        .await
        .0,
        400
    );

    let events = sqlx::query_scalar::<_, Value>("SELECT payload FROM room_events WHERE room_id=$1")
        .bind(room_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .all(|value| value.get("body").is_none()
                && value.to_string().find(private_text).is_none())
    );

    // Independent operator sessions concurrently publishing different
    // candidates must serialize on the room lock, leaving exactly one active.
    let next_a = Uuid::new_v4();
    let next_b = Uuid::new_v4();
    for candidate_id in [next_a, next_b] {
        let next_candidate = topic_candidate(
            candidate_id,
            conversation,
            room_id,
            session,
            "parallel candidate",
        );
        assert_eq!(
            private_topics_json(
                &private,
                "POST",
                "/internal/v1/topics/candidates",
                next_candidate
            )
            .await
            .0,
            202
        );
    }
    let second_operator = boot(&app).await;
    login(&app, &second_operator).await;
    let path_a = format!("{root}/questions/{next_a}/publish");
    let path_b = format!("{root}/questions/{next_b}/publish");
    let (published_a, published_b) = tokio::join!(
        call(&app, &speaker, "POST", &path_a, json!({})),
        call(&app, &second_operator, "POST", &path_b, json!({})),
    );
    assert_eq!(published_a.0, 200);
    assert_eq!(published_b.0, 200);
    let active_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM published_questions WHERE room_id=$1 AND active=true",
    )
    .bind(room_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active_count, 1);
    finish(pool, admin, schema).await;
}

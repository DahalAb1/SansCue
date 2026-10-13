use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sessions_service::{STARTUP_TIMEOUT, app::router, initialize, pool_options};
use sqlx::postgres::PgConnectOptions;
use std::{
    net::TcpListener,
    process::{Command, Stdio},
    str::FromStr,
    time::{Duration, Instant},
};
use tower::ServiceExt;

async fn check(app: &Router, path: &str, status: StatusCode, body: Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["content-type"], "application/json");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), body);
}

#[tokio::test]
async fn unavailable_database_preserves_liveness_and_hides_details() {
    let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "postgres://private:do-not-leak@{}/sessions",
        unavailable.local_addr().unwrap()
    );
    let pool = pool_options().connect_lazy(&url).unwrap();
    let app = router(pool);
    check(&app, "/healthz", StatusCode::OK, json!({"status":"ok"})).await;
    let start = Instant::now();
    check(
        &app,
        "/readyz",
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"status":"not_ready"}),
    )
    .await;
    assert!(start.elapsed() < Duration::from_secs(3));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn database_migrations_readiness_pool_timeout_and_recovery() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must point to a disposable PostgreSQL 17 database; DB tests are never skipped");
    let options = PgConnectOptions::from_str(&url).unwrap();
    let pool = initialize(options.clone()).await.unwrap();
    let app = router(pool.clone());
    check(&app, "/readyz", StatusCode::OK, json!({"status":"ready"})).await;
    let applied: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = 1 AND success")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(applied, 1);
    // Repeat startup proves migrations are safely idempotent.
    initialize(options).await.unwrap().close().await;
    let mut held = Vec::new();
    for _ in 0..5 {
        held.push(pool.acquire().await.unwrap());
    }
    let start = Instant::now();
    check(
        &app,
        "/readyz",
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"status":"not_ready"}),
    )
    .await;
    assert!(start.elapsed() >= Duration::from_millis(1900));
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "readiness must bound acquisition too"
    );
    check(&app, "/healthz", StatusCode::OK, json!({"status":"ok"})).await;
    drop(held);
    check(&app, "/readyz", StatusCode::OK, json!({"status":"ready"})).await;
    pool.close().await;
}

#[test]
fn startup_failure_is_bounded_nonzero_and_never_binds() {
    // A TCP endpoint that accepts no PostgreSQL handshake exercises the total bound.
    let unavailable = TcpListener::bind("127.0.0.1:0").unwrap();
    let http = TcpListener::bind("127.0.0.1:0").unwrap();
    let http_address = http.local_addr().unwrap();
    drop(http);
    let password = "sessions-test-only-do-not-leak-7f3a";
    let url = format!(
        "postgres://private:{password}@{}/sessions",
        unavailable.local_addr().unwrap()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_sessions-service"))
        .env("DATABASE_URL", &url)
        .env("HTTP_HOST", "127.0.0.1")
        .env("HTTP_PORT", http_address.port().to_string())
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        assert!(
            std::net::TcpStream::connect_timeout(&http_address, Duration::from_millis(50)).is_err(),
            "HTTP bound before database initialization"
        );
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            let output = child.wait_with_output().unwrap();
            let logs = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                !logs.contains(password),
                "logs exposed the database password"
            );
            assert!(
                !logs.contains(&url),
                "logs exposed the complete database URL"
            );
            break;
        }
        if start.elapsed() > STARTUP_TIMEOUT + Duration::from_secs(2) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("startup exceeded total bound");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[tokio::test]
async fn migration_failure_exits_before_binding() {
    // Isolated schema makes migration corruption safe for the other DB test.
    use sqlx::Executor;
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL is required for migration failure verification");
    let admin = pool_options().connect(&url).await.unwrap();
    let schema = format!("bootstrap_failure_{}", std::process::id());
    admin
        .execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let isolated = initialize(options).await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = decode('00', 'hex') WHERE version = 1")
        .execute(&isolated)
        .await
        .unwrap();
    isolated.close().await;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut child = Command::new(env!("CARGO_BIN_EXE_sessions-service"))
        .env("DATABASE_URL", &url)
        .env("PGOPTIONS", format!("-c search_path={schema}"))
        .env("HTTP_HOST", "127.0.0.1")
        .env("HTTP_PORT", address.port().to_string())
        .env("RUST_LOG", "info")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let outcome = loop {
        if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok() {
            break Err("listener bound despite invalid migrations");
        }
        if let Some(status) = child.try_wait().unwrap() {
            break Ok(status);
        }
        if start.elapsed() > STARTUP_TIMEOUT + Duration::from_secs(2) {
            break Err("migration failure exceeded startup bound");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    if outcome.is_err() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    admin
        .execute(format!("DROP SCHEMA {schema} CASCADE").as_str())
        .await
        .unwrap();
    admin.close().await;
    assert!(!outcome.unwrap().success());
    let output = child.wait_with_output().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("database migration failed"));
}

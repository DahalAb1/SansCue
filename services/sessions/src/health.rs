use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use sqlx::PgPool;
use std::time::Duration;

pub const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
}

pub async fn live() -> Json<Health> {
    Json(Health { status: "ok" })
}

pub async fn ready(State(pool): State<PgPool>) -> (StatusCode, Json<Health>) {
    match tokio::time::timeout(
        READINESS_TIMEOUT,
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&pool),
    )
    .await
    {
        Ok(Ok(1)) => (StatusCode::OK, Json(Health { status: "ready" })),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(Health {
                status: "not_ready",
            }),
        ),
    }
}

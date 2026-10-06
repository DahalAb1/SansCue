use axum::{Router, routing::get};
use sqlx::PgPool;

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/healthz", get(crate::health::live))
        .route("/readyz", get(crate::health::ready))
        .with_state(pool)
}

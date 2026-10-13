use axum::{Router, routing::get};
use sqlx::PgPool;

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/healthz", get(crate::health::live))
        .route("/readyz", get(crate::health::ready))
        .with_state(pool)
}

pub fn configured_router(pool: PgPool, security: crate::security::Security) -> Router {
    let access = crate::access::Access { pool, security };
    router(access.pool.clone())
        .merge(crate::access::router(access.clone()))
        .merge(crate::live::router(access))
}

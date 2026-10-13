pub mod access;
pub mod app;
pub mod config;
pub mod health;
pub mod live;
pub mod security;

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::time::Duration;

pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_CONNECTIONS: u32 = 5;
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .acquire_timeout(ACQUIRE_TIMEOUT)
}

/// Connection and migrations share one bounded window, before the HTTP listener binds.
/// Errors are deliberately sanitized rather than logging database details/credentials.
pub async fn initialize(database: PgConnectOptions) -> Result<PgPool, &'static str> {
    tokio::time::timeout(STARTUP_TIMEOUT, async {
        let pool = pool_options()
            .connect_with(database)
            .await
            .map_err(|_| "database initialization failed")?;
        MIGRATOR
            .run(&pool)
            .await
            .map_err(|_| "database migration failed")?;
        Ok(pool)
    })
    .await
    .map_err(|_| "database startup timed out")?
}

use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, time::Duration};
use topics_and_questions::{App, Delivery, router};
#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("topics-and-questions: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
async fn run() -> Result<(), &'static str> {
    let db = std::env::var("TOPICS_DATABASE_URL").map_err(|_| "TOPICS_DATABASE_URL is required")?;
    let bee = std::env::var("TOPICS_BEE_SERVICE_TOKEN")
        .map_err(|_| "TOPICS_BEE_SERVICE_TOKEN is required")?;
    let sessions =
        std::env::var("SESSIONS_INTERNAL_URL").map_err(|_| "SESSIONS_INTERNAL_URL is required")?;
    let sessions_token = std::env::var("SESSIONS_TOPICS_SERVICE_TOKEN")
        .map_err(|_| "SESSIONS_TOPICS_SERVICE_TOKEN is required")?;
    let pool = tokio::time::timeout(Duration::from_secs(15), async {
        let p = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&db)
            .await
            .map_err(|_| "topics database connection failed")?;
        sqlx::migrate!("./migrations")
            .run(&p)
            .await
            .map_err(|_| "topics database migration failed")?;
        Ok::<_, &'static str>(p)
    })
    .await
    .map_err(|_| "topics database startup timed out")??;
    let host = std::env::var("HTTP_HOST")
        .unwrap_or_else(|_| "127.0.0.1".into())
        .parse()
        .map_err(|_| "invalid HTTP_HOST")?;
    let port: u16 = std::env::var("HTTP_PORT")
        .unwrap_or_else(|_| "3002".into())
        .parse()
        .map_err(|_| "invalid HTTP_PORT")?;
    if port == 0 {
        return Err("invalid HTTP_PORT");
    };
    let addr = SocketAddr::new(host, port);
    let delivery = Delivery::new(pool.clone(), &sessions, &sessions_token)?;
    let worker = tokio::spawn(delivery.run());
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|_| "HTTP listener bind failed")?;
    let result = axum::serve(
        listener,
        router(App {
            pool: pool.clone(),
            bee_token: bee.into(),
        }),
    )
    .await
    .map_err(|_| "HTTP server failed");
    worker.abort();
    pool.close().await;
    result
}

use sessions_service::{app, config::Config, initialize};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("sessions-service: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), &'static str> {
    let config = Config::from_env()?;
    tracing_subscriber::fmt()
        .with_env_filter(config.log_filter)
        .try_init()
        .map_err(|_| "logging initialization failed")?;
    let pool = initialize(config.database).await?;
    let listener = TcpListener::bind(config.address)
        .await
        .map_err(|_| "HTTP listener bind failed")?;
    tracing::info!(address = %config.address, "sessions-service listening");
    axum::serve(listener, app::router(pool.clone()))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| "HTTP server failed")?;
    pool.close().await;
    Ok(())
}

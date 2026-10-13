use sessions_service::{app, config::Config, initialize};
use std::future::IntoFuture;
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
    let security = sessions_service::security::Security::from_env()?;
    let pool = initialize(config.database).await?;
    let listener = TcpListener::bind(config.address)
        .await
        .map_err(|_| "HTTP listener bind failed")?;
    tracing::info!(
        address = %config.address,
        internal = %config.internal_address,
        "sessions-service listening"
    );
    let internal = TcpListener::bind(config.internal_address)
        .await
        .map_err(|_| "internal listener bind failed")?;
    let public_app = app::configured_router(pool.clone(), security.clone());
    let private_app = sessions_service::access::internal_router(sessions_service::access::Access {
        pool: pool.clone(),
        security,
    });
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut public_rx = shutdown_rx.clone();
    let mut private_rx = shutdown_rx;
    let mut public = std::pin::pin!(
        axum::serve(listener, public_app)
            .with_graceful_shutdown(async move {
                let _ = public_rx.wait_for(|stop| *stop).await;
            })
            .into_future()
    );
    let mut private = std::pin::pin!(
        axum::serve(internal, private_app)
            .with_graceful_shutdown(async move {
                let _ = private_rx.wait_for(|stop| *stop).await;
            })
            .into_future()
    );
    tokio::select! {
        result = &mut public => {
            let _ = shutdown_tx.send(true);
            let _ = private.await;
            result.map_err(|_| "HTTP server failed")?;
        }
        result = &mut private => {
            let _ = shutdown_tx.send(true);
            let _ = public.await;
            result.map_err(|_| "internal server failed")?;
        }
        _ = tokio::signal::ctrl_c() => {
            let _ = shutdown_tx.send(true);
            let _ = public.await;
            let _ = private.await;
        }
    }
    pool.close().await;
    Ok(())
}

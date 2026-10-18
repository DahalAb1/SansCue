use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use topics_and_questions::{
    App, CadencePolicy, Delivery, QuestionGenerator, TopicWindowGenerator,
    claude::{BedrockAuth, ClaudeGenerator},
    router,
};
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
    let cadence_policy = CadencePolicy {
        min_interval_between_questions: duration_setting(
            "TOPICS_MIN_QUESTION_INTERVAL_SECONDS",
            20,
        )?,
        min_words_per_window: usize_setting("TOPICS_MIN_WINDOW_WORDS", 8)?,
        max_window_duration: duration_setting("TOPICS_MAX_WINDOW_SECONDS", 180)?,
        silence_boundary_threshold: duration_setting("TOPICS_SILENCE_BOUNDARY_SECONDS", 5)?,
    };
    let gate = TopicWindowGenerator::new(cadence_policy, "topic-window-stub-v1");
    let bedrock_auth = match (
        optional_setting("AWS_BEARER_TOKEN_BEDROCK")?,
        optional_setting("TOPICS_BEDROCK_CREDENTIALS_FILE")?,
    ) {
        (Some(_), Some(_)) => {
            return Err(
                "set either AWS_BEARER_TOKEN_BEDROCK or TOPICS_BEDROCK_CREDENTIALS_FILE, not both",
            );
        }
        (Some(key), None) => Some(BedrockAuth::ApiKey(key)),
        (None, Some(file)) => Some(BedrockAuth::CredentialsFile(file.into())),
        (None, None) => None,
    };
    let generator: Arc<dyn QuestionGenerator> = match (
        bedrock_auth,
        optional_setting("ANTHROPIC_API_KEY")?,
    ) {
        (Some(_), Some(_)) => {
            return Err("configure either Bedrock or ANTHROPIC_API_KEY, not both");
        }
        (Some(auth), None) => Arc::new(ClaudeGenerator::bedrock(
            gate,
            auth,
            optional_setting("AWS_REGION")?
                .as_deref()
                .unwrap_or("us-east-1"),
            optional_setting("TOPICS_MODEL")?.as_deref(),
            None,
        )?),
        (None, Some(key)) => Arc::new(ClaudeGenerator::anthropic(
            gate,
            &key,
            optional_setting("ANTHROPIC_BASE_URL")?.as_deref(),
            optional_setting("TOPICS_MODEL")?.as_deref(),
        )?),
        (None, None) => {
            eprintln!(
                "topics-and-questions: no model credential is set; using the deterministic placeholder generator"
            );
            Arc::new(gate)
        }
    };
    let delivery = Delivery::new(pool.clone(), &sessions, &sessions_token)?
        .with_generator(generator)
        .with_cadence_policy(cadence_policy);
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

fn optional_setting(name: &str) -> Result<Option<String>, &'static str> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err("invalid Topics model setting"),
    }
}

fn duration_setting(name: &str, default: u64) -> Result<Duration, &'static str> {
    let seconds = match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| "invalid Topics cadence setting")?,
        Err(std::env::VarError::NotPresent) => default,
        Err(_) => return Err("invalid Topics cadence setting"),
    };
    if seconds == 0 {
        return Err("invalid Topics cadence setting");
    }
    Ok(Duration::from_secs(seconds))
}

fn usize_setting(name: &str, default: usize) -> Result<usize, &'static str> {
    let value = match std::env::var(name) {
        Ok(value) => value
            .parse::<usize>()
            .map_err(|_| "invalid Topics cadence setting")?,
        Err(std::env::VarError::NotPresent) => default,
        Err(_) => return Err("invalid Topics cadence setting"),
    };
    if value == 0 {
        return Err("invalid Topics cadence setting");
    }
    Ok(value)
}

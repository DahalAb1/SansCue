use bee_connection::{
    adapter::{BeeAdapter, ReplayAdapter},
    consumer::Consumer,
    domain::Binding,
    store::Store,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("bee-connection: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--worker"] {
        let url = std::env::var("BEE_DATABASE_URL").map_err(|_| "BEE_DATABASE_URL is required")?;
        let sessions_url = std::env::var("SESSIONS_INTERNAL_URL")
            .map_err(|_| "SESSIONS_INTERNAL_URL is required")?;
        let token = std::env::var("SESSIONS_BEE_SERVICE_TOKEN")
            .map_err(|_| "SESSIONS_BEE_SERVICE_TOKEN is required")?;
        let topics_url =
            std::env::var("TOPICS_INTERNAL_URL").map_err(|_| "TOPICS_INTERNAL_URL is required")?;
        let topics_token = std::env::var("TOPICS_BEE_SERVICE_TOKEN")
            .map_err(|_| "TOPICS_BEE_SERVICE_TOKEN is required")?;
        let store = Store::connect(&url).await?;
        let consumer = Consumer::new(&sessions_url, &token, store.clone())?;
        let topics = bee_connection::topics_consumer::TopicsConsumer::new(
            &topics_url,
            &topics_token,
            store.clone(),
        )?;
        tokio::select! {
            result=consumer.run()=>result,
            result=topics.run()=>result,
            _=tokio::signal::ctrl_c()=>{store.close().await;Ok(())}
        }?;
        return Ok(());
    }
    // Local trusted replay harness, independent of live device access.
    if args.len() != 4 {
        return Err("usage: bee-connection RECORDING CONVERSATION_UUID ROOM_UUID SESSION_UUID");
    }
    let parse_id = |value: &str| Uuid::parse_str(value).map_err(|_| "invalid binding UUID");
    let binding = Binding {
        conversation_id: parse_id(&args[1])?,
        source_conversation_id: None,
        room_id: parse_id(&args[2])?,
        session_id: parse_id(&args[3])?,
    };
    let bytes = std::fs::read(&args[0]).map_err(|_| "recording read failed")?;
    let mut adapter =
        ReplayAdapter::from_recording(&bytes).map_err(|_| "invalid SansCue recording envelope")?;
    let url = std::env::var("BEE_DATABASE_URL")
        .map_err(|_| "BEE_DATABASE_URL is required and must name the Bee-only database")?;
    let store = Store::connect(&url).await?;
    store
        .open(binding.conversation_id, adapter.capabilities())
        .await?;
    store.bind(binding.clone()).await?;
    while let Some(observation) = adapter.next_observation() {
        store.ingest(binding.conversation_id, observation).await?;
    }
    let state = store.load(binding.conversation_id).await?;
    println!(
        "observations={} events={} gap_intervals={}",
        state.observations().len(),
        state.events().len(),
        state.gaps().len()
    );
    store.close().await;
    Ok(())
}

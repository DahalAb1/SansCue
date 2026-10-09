use bee_connection::{
    domain::*,
    pipeline::*,
    store::{CommandOutcome, Store},
};
use uuid::Uuid;

#[tokio::test]
async fn persistence_restart_concurrent_deduplication_binding_and_raw_quarantine() {
    let url = std::env::var("BEE_TEST_DATABASE_URL").expect("BEE_TEST_DATABASE_URL must name a disposable Bee-only PostgreSQL database; this test never skips");
    let store = Store::connect(&url).await.unwrap();
    let id = Uuid::new_v4();
    let capabilities = Capabilities {
        stable_ids: true,
        contiguous_sequence: true,
    };
    store.open(id, capabilities).await.unwrap();
    store.open(id, capabilities).await.unwrap();
    assert!(store.open(id, Capabilities::default()).await.is_err());
    let binding = Binding {
        conversation_id: id,
        source_conversation_id: None,
        room_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
    };
    store.bind(binding.clone()).await.unwrap();
    store.bind(binding.clone()).await.unwrap();
    let mut changed = binding.clone();
    changed.room_id = Uuid::new_v4();
    assert!(store.bind(changed).await.is_err());
    let observation = Observation {
        received_at: "2026-10-08T00:00:00Z".parse().unwrap(),
        raw: b"original raw".to_vec(),
        decoded: Ok(Signal::Transcript(Transcript {
            text: "hello".into(),
            source_conversation_id: None,
            source_id: Some("synthetic-1".into()),
            source_sequence: Some(1),
        })),
    };
    let (first, second) = tokio::join!(
        store.ingest(id, observation.clone()),
        store.ingest(id, observation.clone())
    );
    let results = [first.unwrap(), second.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|d| matches!(d, Disposition::Accepted { .. }))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|d| matches!(d, Disposition::Duplicate { .. }))
            .count(),
        1
    );
    let malformed = Observation {
        raw: vec![255, 0],
        decoded: Err("bad encoding".into()),
        ..observation.clone()
    };
    store.ingest(id, malformed.clone()).await.unwrap();
    store
        .ingest(
            id,
            Observation {
                decoded: Ok(Signal::Disconnected),
                ..observation.clone()
            },
        )
        .await
        .unwrap();
    let before = store.load(id).await.unwrap();
    store.close().await;
    let restarted = Store::connect(&url).await.unwrap();
    assert_eq!(restarted.load(id).await.unwrap(), before);
    assert!(matches!(
        restarted.ingest(id, observation).await.unwrap(),
        Disposition::Duplicate { .. }
    ));
    let after = restarted.load(id).await.unwrap();
    assert_eq!(after.events().len(), 1);
    assert_eq!(after.events()[0].binding, binding);
    assert_eq!(after.observations()[2].observation, malformed);
    assert_eq!(after.observations().last().unwrap().ordinal, 5);
    assert_eq!(after.gaps()[0].status, GapStatus::Suspected);
    let at = "2026-10-08T00:00:07Z".parse().unwrap();
    restarted
        .ingest(
            id,
            Observation {
                received_at: at,
                raw: vec![],
                decoded: Ok(Signal::Reconnected),
            },
        )
        .await
        .unwrap();
    restarted
        .ingest(
            id,
            Observation {
                received_at: at,
                raw: b"second".to_vec(),
                decoded: Ok(Signal::Transcript(Transcript {
                    text: "second".into(),
                    source_conversation_id: None,
                    source_id: Some("synthetic-2".into()),
                    source_sequence: Some(2),
                })),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        restarted.load(id).await.unwrap().gaps()[0].status,
        GapStatus::Recovered
    );
    restarted.close().await;
}

#[tokio::test]
async fn room_revision_fence_rejects_delayed_bind_after_unbind() {
    let url=std::env::var("BEE_TEST_DATABASE_URL").expect("BEE_TEST_DATABASE_URL must name a disposable Bee-only PostgreSQL database; this test never skips");
    let store = Store::connect(&url).await.unwrap();
    let conversation = Uuid::new_v4();
    let room = Uuid::new_v4();
    let session = Uuid::new_v4();
    let binding = Binding {
        conversation_id: conversation,
        source_conversation_id: Some("synthetic-conversation".into()),
        room_id: room,
        session_id: session,
    };
    store
        .ensure_command_conversation(conversation)
        .await
        .unwrap();
    let unbind = Uuid::new_v4();
    let unbind_result = store
        .apply_command(unbind, 2, "unbind", binding.clone())
        .await
        .unwrap();
    assert_eq!(unbind_result, CommandOutcome::Applied);
    assert!(store.load(conversation).await.unwrap().binding().is_none());
    // Bind r1 remained pending while a later unbind r2 was applied.
    let stale = store
        .apply_command(Uuid::new_v4(), 1, "bind", binding.clone())
        .await
        .unwrap();
    assert_eq!(stale, CommandOutcome::Stale);
    assert!(store.load(conversation).await.unwrap().binding().is_none());
    // The caller can ack that stale row (without publishing status) and proceed.
    let next = store
        .apply_command(Uuid::new_v4(), 3, "bind", binding.clone())
        .await
        .unwrap();
    assert_eq!(next, CommandOutcome::Applied);
    assert!(store.load(conversation).await.unwrap().binding().is_some());
    assert_eq!(
        store
            .apply_command(unbind, 2, "unbind", binding)
            .await
            .unwrap(),
        CommandOutcome::Stale
    );
    assert!(store.load(conversation).await.unwrap().binding().is_some());
    store.close().await;
}

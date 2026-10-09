use chrono::Utc;
use serde_json::Value;
use sqlx::{Row, postgres::PgPoolOptions};
use std::sync::Arc;
use topics_and_questions::{App, Binding, TranscriptEvent, router};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn authenticated_ingress_is_idempotent_and_rejects_identity_reuse() {
    let url=std::env::var("TOPICS_TEST_DATABASE_URL").expect("TOPICS_TEST_DATABASE_URL must name a disposable Topics-only PostgreSQL database; this test never skips");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect Topics test DB");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate Topics test DB");
    let event = TranscriptEvent {
        schema_version: 1,
        event_id: Uuid::new_v4(),
        binding: Binding {
            conversation_id: Uuid::new_v4(),
            source_conversation_id: Some("synthetic-source".into()),
            room_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
        },
        ingest_ordinal: 1,
        received_at: Utc::now(),
        source_id: Some("synthetic-event".into()),
        source_sequence: Some(u64::MAX),
        text: "Synthetic transcript passage.".into(),
    };
    let app = router(App {
        pool: pool.clone(),
        bee_token: Arc::from("test-bee-token"),
    });
    let make = |body: Value, auth: bool| {
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/internal/v1/transcript-events")
            .header("content-type", "application/json");
        if auth {
            req = req.header("authorization", "Bearer test-bee-token")
        };
        req.body(axum::body::Body::from(body.to_string())).unwrap()
    };
    let unauthorized = app
        .clone()
        .oneshot(make(serde_json::to_value(&event).unwrap(), false))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), axum::http::StatusCode::UNAUTHORIZED);
    let accepted = app
        .clone()
        .oneshot(make(serde_json::to_value(&event).unwrap(), true))
        .await
        .unwrap();
    assert_eq!(accepted.status(), axum::http::StatusCode::ACCEPTED);
    let duplicate = app
        .clone()
        .oneshot(make(serde_json::to_value(&event).unwrap(), true))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), axum::http::StatusCode::ACCEPTED);
    let mut conflicting = event.clone();
    conflicting.text.push_str(" changed");
    let conflict = app
        .oneshot(make(serde_json::to_value(conflicting).unwrap(), true))
        .await
        .unwrap();
    assert_eq!(conflict.status(), axum::http::StatusCode::CONFLICT);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM transcript_events WHERE event_id=$1")
        .bind(event.event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let jobs = sqlx::query("SELECT status FROM generation_jobs WHERE event_id=$1")
        .bind(event.event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(jobs.get::<String, _>("status"), "pending");
    sqlx::query("DELETE FROM generation_jobs WHERE event_id=$1")
        .bind(event.event_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM transcript_events WHERE event_id=$1")
        .bind(event.event_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

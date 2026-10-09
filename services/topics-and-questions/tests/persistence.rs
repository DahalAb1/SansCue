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

#[derive(Clone)]
struct FakeSessions {
    stale: Uuid,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

async fn candidate_callback(
    axum::extract::State(state): axum::extract::State<FakeSessions>,
    axum::Json(candidate): axum::Json<topics_and_questions::Candidate>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    state
        .calls
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if candidate.candidate_id == state.stale {
        return (
            axum::http::StatusCode::GONE,
            axum::Json(serde_json::json!({"code":"topic_candidate_stale"})),
        )
            .into_response();
    }
    (
        axum::http::StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({
            "accepted":true,
            "candidate_id":candidate.candidate_id
        })),
    )
        .into_response()
}

#[tokio::test]
async fn terminal_stale_candidate_does_not_block_later_outbox_rows() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use topics_and_questions::{Candidate, Delivery, Evidence};

    let url=std::env::var("TOPICS_TEST_DATABASE_URL").expect("TOPICS_TEST_DATABASE_URL must name a disposable Topics-only PostgreSQL database; this test never skips");
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect Topics test DB");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate Topics test DB");

    let stale_id = Uuid::new_v4();
    let accepted_id = Uuid::new_v4();
    let mut ids = Vec::new();
    for (index, event_id) in [stale_id, accepted_id].into_iter().enumerate() {
        let conversation_id = Uuid::new_v4();
        let room_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let received_at = Utc::now();
        let evidence = Evidence {
            conversation_id,
            ingest_ordinal: index as u64 + 1,
            received_at,
            excerpt: format!("Synthetic passage {index}"),
        };
        sqlx::query("INSERT INTO transcript_events(event_id,schema_version,conversation_id,room_id,session_id,source_conversation_id,ingest_ordinal,received_at,source_id,source_sequence,text,projection_hash) VALUES($1,1,$2,$3,$4,NULL,$5,$6,NULL,NULL,$7,$8)")
            .bind(event_id).bind(conversation_id).bind(room_id).bind(session_id).bind(index as i64 + 1).bind(received_at).bind(&evidence.excerpt).bind(vec![0_u8;32])
            .execute(&pool).await.expect("insert test transcript");
        let candidate = Candidate {
            candidate_id: event_id,
            event_id,
            room_id,
            session_id,
            text: format!("What is the implication of passage {index}?"),
            generator_version: "stub-v1".into(),
            evidence,
            published: false,
        };
        sqlx::query("INSERT INTO question_candidates(candidate_id,event_id,room_id,session_id,text,generator_version,evidence) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(candidate.candidate_id).bind(candidate.event_id).bind(candidate.room_id).bind(candidate.session_id).bind(&candidate.text).bind(&candidate.generator_version).bind(sqlx::types::Json(&candidate.evidence))
            .execute(&pool).await.expect("insert test candidate");
        sqlx::query("INSERT INTO candidate_outbox(candidate_id,created_at) VALUES($1,now()+($2 * interval '1 second'))")
            .bind(event_id).bind(index as i64).execute(&pool).await.expect("insert test outbox");
        ids.push(event_id);
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new()
        .route(
            "/internal/v1/topics/candidates",
            axum::routing::post(candidate_callback),
        )
        .with_state(FakeSessions {
            stale: stale_id,
            calls: calls.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let delivery = Delivery::new(
        pool.clone(),
        &format!("http://{address}"),
        "test-sessions-token",
    )
    .unwrap();
    assert_eq!(delivery.poll_once().await.unwrap(), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let states = sqlx::query("SELECT candidate_id,delivered_at,terminal_at,terminal_code FROM candidate_outbox WHERE candidate_id=ANY($1)")
        .bind(&ids).fetch_all(&pool).await.unwrap();
    assert_eq!(states.len(), 2);
    for row in states {
        let id: Uuid = row.get("candidate_id");
        if id == stale_id {
            assert!(
                row.get::<Option<chrono::DateTime<Utc>>, _>("terminal_at")
                    .is_some()
            );
            assert_eq!(
                row.get::<String, _>("terminal_code"),
                "topic_candidate_stale"
            );
            assert!(
                row.get::<Option<chrono::DateTime<Utc>>, _>("delivered_at")
                    .is_none()
            );
        } else {
            assert!(
                row.get::<Option<chrono::DateTime<Utc>>, _>("delivered_at")
                    .is_some()
            );
            assert!(
                row.get::<Option<chrono::DateTime<Utc>>, _>("terminal_at")
                    .is_none()
            );
        }
    }
    assert_eq!(delivery.poll_once().await.unwrap(), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    server.abort();
    sqlx::query("DELETE FROM candidate_outbox WHERE candidate_id=ANY($1)")
        .bind(&ids)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM question_candidates WHERE candidate_id=ANY($1)")
        .bind(&ids)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM transcript_events WHERE event_id=ANY($1)")
        .bind(&ids)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

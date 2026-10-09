use crate::{
    domain::*,
    pipeline::{Disposition, Pipeline},
};
use sqlx::{PgPool, postgres::PgPoolOptions, types::Json};
use std::time::Duration;
use uuid::Uuid;

/// First-step aggregate store. Row locks serialize writers per conversation;
/// state and raw observations commit together. Not a large-stream storage design.
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    pub async fn connect(url: &str) -> Result<Self, &'static str> {
        tokio::time::timeout(Duration::from_secs(15), async {
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .acquire_timeout(Duration::from_secs(5))
                .connect(url)
                .await
                .map_err(|_| "Bee database connection failed")?;
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .map_err(|_| "Bee database migration failed")?;
            Ok(Self { pool })
        })
        .await
        .map_err(|_| "Bee database startup timed out")?
    }

    /// Reopening must use the same capabilities; no silent recovery-policy changes.
    pub async fn open(&self, id: Uuid, capabilities: Capabilities) -> Result<(), &'static str> {
        if id.is_nil() {
            return Err("invalid conversation ID");
        }
        sqlx::query("INSERT INTO bee_conversations (conversation_id, state) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(id).bind(Json(Pipeline::new(id, capabilities))).execute(&self.pool).await.map_err(|_| "Bee conversation creation failed")?;
        if self.load(id).await?.capabilities != capabilities {
            return Err("conversation capabilities conflict");
        }
        Ok(())
    }

    pub async fn load(&self, id: Uuid) -> Result<Pipeline, &'static str> {
        let state: Json<Pipeline> =
            sqlx::query_scalar("SELECT state FROM bee_conversations WHERE conversation_id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(|_| "Bee conversation load failed")?;
        Ok(state.0)
    }

    pub async fn bind(&self, binding: Binding) -> Result<(), &'static str> {
        self.update(binding.conversation_id, |pipeline| pipeline.bind(binding))
            .await
    }

    pub async fn ingest(
        &self,
        id: Uuid,
        observation: Observation,
    ) -> Result<Disposition, &'static str> {
        self.update(
            id,
            |pipeline| Ok(pipeline.ingest(observation, Uuid::new_v4)),
        )
        .await
    }

    async fn update<T>(
        &self,
        id: Uuid,
        change: impl FnOnce(&mut Pipeline) -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| "Bee transaction failed")?;
        let state: Json<Pipeline> = sqlx::query_scalar(
            "SELECT state FROM bee_conversations WHERE conversation_id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Bee conversation lock failed")?;
        let mut pipeline = state.0;
        let result = change(&mut pipeline)?;
        sqlx::query("UPDATE bee_conversations SET state = $2 WHERE conversation_id = $1")
            .bind(id)
            .bind(Json(pipeline))
            .execute(&mut *tx)
            .await
            .map_err(|_| "Bee state write failed")?;
        tx.commit().await.map_err(|_| "Bee commit failed")?;
        Ok(result)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

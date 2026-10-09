use crate::{
    domain::*,
    pipeline::{Disposition, Pipeline},
};
use serde_json::Value;
use sqlx::{PgPool, Row, postgres::PgPoolOptions, types::Json};
use std::time::Duration;
use uuid::Uuid;

/// First-step aggregate store. Row locks serialize writers per conversation;
/// state and raw observations commit together. Not a large-stream storage design.
#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    Applied,
    Stale,
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

    /// Ensure a command-target conversation exists without changing any
    /// previously persisted transport capability assertion.
    pub async fn ensure_command_conversation(&self, id: Uuid) -> Result<(), &'static str> {
        if id.is_nil() {
            return Err("invalid conversation ID");
        }
        sqlx::query("INSERT INTO bee_conversations (conversation_id,state) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind(id).bind(Json(Pipeline::new(id, Capabilities::default())))
            .execute(&self.pool).await.map_err(|_| "Bee conversation creation failed")?;
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

    /// Apply a sessions-authoritative command once. The command receipt and
    /// conversation aggregate are committed atomically in Bee's own database.
    pub async fn apply_command(
        &self,
        command_id: Uuid,
        revision: i64,
        action: &str,
        binding: Binding,
    ) -> Result<CommandOutcome, &'static str> {
        if command_id.is_nil() || revision < 1 || !matches!(action, "bind" | "unbind") {
            return Err("invalid command");
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| "Bee transaction failed")?;
        if let Some(row)=sqlx::query("SELECT conversation_id,room_id,revision,command_type,result FROM bee_command_inbox WHERE command_id=$1")
            .bind(command_id).fetch_optional(&mut *tx).await.map_err(|_| "Bee command receipt load failed")? {
            if row.get::<Uuid,_>("conversation_id") != binding.conversation_id || row.get::<Uuid,_>("room_id") != binding.room_id || row.get::<i64,_>("revision") != revision || row.get::<String,_>("command_type") != action { return Err("command identity conflict"); }
            let recorded_stale=row.get::<Value,_>("result")["stale"] == true;
            let latest:Option<i64>=sqlx::query_scalar("SELECT revision FROM bee_room_revision WHERE room_id=$1")
                .bind(binding.room_id).fetch_optional(&mut *tx).await.map_err(|_| "Bee room revision load failed")?;
            let stale=recorded_stale || latest.is_some_and(|current| current > revision);
            tx.commit().await.map_err(|_| "Bee commit failed")?;
            return Ok(if stale {CommandOutcome::Stale} else {CommandOutcome::Applied});
        }
        sqlx::query("INSERT INTO bee_room_revision(room_id,revision,conversation_id) VALUES($1,0,NULL) ON CONFLICT(room_id) DO NOTHING")
            .bind(binding.room_id).execute(&mut *tx).await.map_err(|_| "Bee room revision initialization failed")?;
        let fence = sqlx::query(
            "SELECT revision,conversation_id FROM bee_room_revision WHERE room_id=$1 FOR UPDATE",
        )
        .bind(binding.room_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Bee room revision lock failed")?;
        let latest: i64 = fence.get("revision");
        let current: Option<Uuid> = fence.get("conversation_id");
        if revision <= latest {
            sqlx::query("INSERT INTO bee_command_inbox(command_id,room_id,conversation_id,revision,command_type,result) VALUES($1,$2,$3,$4,$5,'{\"stale\":true}') ON CONFLICT(command_id) DO NOTHING")
                .bind(command_id).bind(binding.room_id).bind(binding.conversation_id).bind(revision).bind(action).execute(&mut *tx).await.map_err(|_| "Bee stale command receipt write failed")?;
            tx.commit().await.map_err(|_| "Bee commit failed")?;
            return Ok(CommandOutcome::Stale);
        }
        if action == "bind" && current.is_some_and(|id| id != binding.conversation_id) {
            let previous = current.expect("checked above");
            if let Some(old) = sqlx::query_scalar::<_, Json<Pipeline>>(
                "SELECT state FROM bee_conversations WHERE conversation_id=$1 FOR UPDATE",
            )
            .bind(previous)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| "Bee prior conversation load failed")?
            {
                let mut pipeline = old.0;
                pipeline.unbind(previous)?;
                sqlx::query("UPDATE bee_conversations SET state=$2 WHERE conversation_id=$1")
                    .bind(previous)
                    .bind(Json(pipeline))
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| "Bee prior binding close failed")?;
            }
        }
        let state: Json<Pipeline> = sqlx::query_scalar(
            "SELECT state FROM bee_conversations WHERE conversation_id=$1 FOR UPDATE",
        )
        .bind(binding.conversation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| "Bee conversation lock failed")?;
        let mut pipeline = state.0;
        if action == "bind" {
            pipeline.bind(binding.clone())?;
        } else if pipeline.binding().is_some_and(|current| {
            current.room_id != binding.room_id || current.session_id != binding.session_id
        }) {
            return Err("unbind binding mismatch");
        } else {
            pipeline.unbind(binding.conversation_id)?;
        }
        sqlx::query("UPDATE bee_conversations SET state=$2 WHERE conversation_id=$1")
            .bind(binding.conversation_id)
            .bind(Json(pipeline))
            .execute(&mut *tx)
            .await
            .map_err(|_| "Bee state write failed")?;
        sqlx::query("INSERT INTO bee_command_inbox(command_id,room_id,conversation_id,revision,command_type,result) VALUES($1,$2,$3,$4,$5,'{}')")
            .bind(command_id).bind(binding.room_id).bind(binding.conversation_id).bind(revision).bind(action).execute(&mut *tx).await.map_err(|_| "Bee command receipt write failed")?;
        sqlx::query("UPDATE bee_room_revision SET revision=$2,conversation_id=$3,updated_at=now() WHERE room_id=$1")
            .bind(binding.room_id).bind(revision).bind(if action == "bind" {Some(binding.conversation_id)} else {None}).execute(&mut *tx).await.map_err(|_| "Bee room revision update failed")?;
        tx.commit().await.map_err(|_| "Bee commit failed")?;
        Ok(CommandOutcome::Applied)
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

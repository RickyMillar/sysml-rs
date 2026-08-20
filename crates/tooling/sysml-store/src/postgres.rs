//! PostgreSQL storage backend for SysML v2 model snapshots.
//!
//! Requires the `postgres` feature to be enabled.

use crate::{BaselineMeta, Snapshot, SnapshotMeta, StoreError};
use sqlx::postgres::PgPool;
use sysml_core::json::to_json_string;
use sysml_core::ModelGraph;
use sysml_id::{CommitId, ProjectId};

/// PostgreSQL-backed store.
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    /// Create a new PostgreSQL store.
    pub async fn new(database_url: &str) -> Result<Self, StoreError> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        Ok(PostgresStore { pool })
    }

    /// Initialize the database schema.
    pub async fn init_schema(&self) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS snapshots (
                project_id TEXT NOT NULL,
                commit_id TEXT NOT NULL,
                parent_id TEXT,
                message TEXT NOT NULL,
                timestamp BIGINT NOT NULL,
                data JSONB NOT NULL,
                PRIMARY KEY (project_id, commit_id)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_snapshots_project
            ON snapshots (project_id, timestamp DESC)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        // Baselines are immutable named pointers to commits (see the
        // `Store` trait docs): the PK forbids re-creating a name, and there
        // is deliberately no UPDATE or DELETE path. `seq` preserves creation
        // order for newest-first listing (created_at has only second
        // granularity). Postgres never evicts snapshots, so the in-memory
        // store's eviction exemption is inherently satisfied here.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS baselines (
                seq BIGSERIAL,
                project_id TEXT NOT NULL,
                name TEXT NOT NULL,
                commit_id TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                git_sha TEXT,
                git_dirty BOOLEAN,
                git_branch TEXT,
                PRIMARY KEY (project_id, name)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        // B6 additive migration for pre-provenance deployments. All three
        // columns are NULLABLE with no default: NULL = "provenance not
        // captured" (old baselines, non-git workspaces) — a DEFAULT false
        // on git_dirty would fabricate a fact for rows where it is unknown.
        for column in [
            "ADD COLUMN IF NOT EXISTS git_sha TEXT",
            "ADD COLUMN IF NOT EXISTS git_dirty BOOLEAN",
            "ADD COLUMN IF NOT EXISTS git_branch TEXT",
        ] {
            sqlx::query(&format!("ALTER TABLE baselines {column}"))
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::DatabaseError(e.to_string()))?;
        }

        Ok(())
    }

    /// Store a snapshot asynchronously.
    pub async fn put_snapshot_async(
        &self,
        project: &ProjectId,
        meta: SnapshotMeta,
        graph: &ModelGraph,
    ) -> Result<(), StoreError> {
        let data = to_json_string(graph);

        sqlx::query(
            r#"
            INSERT INTO snapshots (project_id, commit_id, parent_id, message, timestamp, data)
            VALUES ($1, $2, $3, $4, $5, $6::jsonb)
            "#,
        )
        .bind(project.as_str())
        .bind(meta.commit.as_str())
        .bind(meta.parent.as_ref().map(|p| p.as_str().to_string()))
        .bind(&meta.message)
        .bind(meta.timestamp as i64)
        .bind(&data)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("duplicate key") {
                StoreError::Conflict(format!("commit {} already exists", meta.commit))
            } else {
                StoreError::DatabaseError(e.to_string())
            }
        })?;

        Ok(())
    }

    /// Get a snapshot asynchronously.
    pub async fn get_snapshot_async(
        &self,
        project: &ProjectId,
        commit: &CommitId,
    ) -> Result<Option<Snapshot>, StoreError> {
        let row: Option<(String, Option<String>, String, i64, String)> = sqlx::query_as(
            r#"
            SELECT commit_id, parent_id, message, timestamp, data::text
            FROM snapshots
            WHERE project_id = $1 AND commit_id = $2
            "#,
        )
        .bind(project.as_str())
        .bind(commit.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        match row {
            Some((commit_id, parent_id, message, timestamp, data)) => {
                let mut meta = SnapshotMeta::new(CommitId::new(commit_id), message)
                    .with_timestamp(timestamp as u64);
                if let Some(parent) = parent_id {
                    meta = meta.with_parent(CommitId::new(parent));
                }
                Ok(Some(Snapshot { meta, data }))
            }
            None => Ok(None),
        }
    }

    /// Get the latest commit ID asynchronously.
    pub async fn latest_async(
        &self,
        project: &ProjectId,
    ) -> Result<Option<CommitId>, StoreError> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT commit_id
            FROM snapshots
            WHERE project_id = $1
            ORDER BY timestamp DESC
            LIMIT 1
            "#,
        )
        .bind(project.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        Ok(row.map(|(id,)| CommitId::new(id)))
    }

    /// Create a named, immutable baseline pointing at an existing commit.
    ///
    /// Same contract as [`crate::Store::create_baseline`]: fails with
    /// [`StoreError::Conflict`] if the name is already taken in this project
    /// and with [`StoreError::CommitNotFound`] if the commit has no stored
    /// snapshot.
    pub async fn create_baseline_async(
        &self,
        project: &ProjectId,
        name: &str,
        commit: &CommitId,
        provenance: Option<crate::GitProvenance>,
    ) -> Result<(), StoreError> {
        let (exists,): (bool,) = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM snapshots
                WHERE project_id = $1 AND commit_id = $2
            )
            "#,
        )
        .bind(project.as_str())
        .bind(commit.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        if !exists {
            return Err(StoreError::CommitNotFound(format!(
                "cannot baseline {commit}: no snapshot stored for it"
            )));
        }

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        sqlx::query(
            r#"
            INSERT INTO baselines (project_id, name, commit_id, created_at, git_sha, git_dirty, git_branch)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(project.as_str())
        .bind(name)
        .bind(commit.as_str())
        .bind(created_at as i64)
        .bind(provenance.as_ref().map(|p| p.sha.clone()))
        .bind(provenance.as_ref().map(|p| p.dirty))
        .bind(provenance.as_ref().and_then(|p| p.branch.clone()))
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if e.to_string().contains("duplicate key") {
                StoreError::Conflict(format!(
                    "baseline '{name}' already exists (baselines are immutable — pick a new name)"
                ))
            } else {
                StoreError::DatabaseError(e.to_string())
            }
        })?;

        Ok(())
    }

    /// Resolve a baseline name to its commit. `Ok(None)` means the baseline
    /// was never created.
    pub async fn get_baseline_async(
        &self,
        project: &ProjectId,
        name: &str,
    ) -> Result<Option<CommitId>, StoreError> {
        let row: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT commit_id
            FROM baselines
            WHERE project_id = $1 AND name = $2
            "#,
        )
        .bind(project.as_str())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        Ok(row.map(|(id,)| CommitId::new(id)))
    }

    /// List baselines for a project (most recently created first).
    pub async fn list_baselines_async(
        &self,
        project: &ProjectId,
    ) -> Result<Vec<BaselineMeta>, StoreError> {
        let rows: Vec<(String, String, i64, Option<String>, Option<bool>, Option<String>)> =
            sqlx::query_as(
                r#"
            SELECT name, commit_id, created_at, git_sha, git_dirty, git_branch
            FROM baselines
            WHERE project_id = $1
            ORDER BY seq DESC
            "#,
            )
            .bind(project.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::DatabaseError(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|(name, commit_id, created_at, sha, dirty, branch)| BaselineMeta {
                name,
                commit: CommitId::new(commit_id),
                created_at: created_at as u64,
                // sha+dirty are captured together; a row with one but not
                // the other would be malformed — surface as no provenance
                // rather than inventing the missing half.
                provenance: match (sha, dirty) {
                    (Some(sha), Some(dirty)) => Some(crate::GitProvenance { sha, dirty, branch }),
                    _ => None,
                },
            })
            .collect())
    }
}

/// Create an in-memory store (fallback when postgres feature is disabled).
pub fn create_in_memory_store() -> crate::InMemoryStore {
    crate::InMemoryStore::new()
}

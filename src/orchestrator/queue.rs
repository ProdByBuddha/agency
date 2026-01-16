//! Durable Task Queue backed by SQLite (rusqlite)
//! 
//! Provides a persistent queue for agent tasks, ensuring that
//! intentions and planned actions survive process restarts.
//! Uses rusqlite (synchronous) wrapped in spawn_blocking for async compatibility.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::task;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Retrying,
}

impl ToString for TaskStatus {
    fn to_string(&self) -> String {
        match self {
            TaskStatus::Pending => "pending".to_string(),
            TaskStatus::Running => "running".to_string(),
            TaskStatus::Completed => "completed".to_string(),
            TaskStatus::Failed => "failed".to_string(),
            TaskStatus::Retrying => "retrying".to_string(),
        }
    }
}

impl From<String> for TaskStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "pending" => TaskStatus::Pending,
            "running" => TaskStatus::Running,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "retrying" => TaskStatus::Retrying,
            _ => TaskStatus::Pending, // Default fallback
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub kind: String,
    pub payload: String, // JSON payload
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub attempts: i32,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct TaskQueue {
    db_path: PathBuf,
    // We don't hold an open connection across await points because Connection is not Send/Sync.
    // Instead, we open a new connection or pool in spawn_blocking.
    // For sqlite, opening connection is fast.
}

impl TaskQueue {
    /// Initialize the task queue, creating the database file if needed
    pub async fn new(db_path: impl AsRef<Path>) -> Result<Self> {
        let path = db_path.as_ref().to_path_buf();
        let path_clone = path.clone();

        task::spawn_blocking(move || {
            let conn = Connection::open(&path_clone)?;
            
            conn.execute(
                r#"
                CREATE TABLE IF NOT EXISTS tasks (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL,
                    payload TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    attempts INTEGER DEFAULT 0,
                    last_error TEXT
                );
                "#,
                [],
            )?;
            
            conn.execute("CREATE INDEX IF NOT EXISTS idx_status ON tasks(status);", [])?;
            conn.execute("CREATE INDEX IF NOT EXISTS idx_created_at ON tasks(created_at);", [])?;

            Ok::<_, anyhow::Error>(())
        }).await??;

        Ok(Self { db_path: path })
    }

    /// Enqueue a new task
    pub async fn enqueue(&self, kind: &str, payload: impl Serialize) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let payload_json = serde_json::to_string(&payload)?;
        let kind_str = kind.to_string();
        let path = self.db_path.clone();

        task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let now = Utc::now().to_rfc3339();

            conn.execute(
                "INSERT INTO tasks (id, kind, payload, status, created_at, updated_at, attempts) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![&id, &kind_str, &payload_json, "pending", &now, &now],
            )?;
            Ok::<_, anyhow::Error>(id)
        }).await?
    }

    /// Dequeue the next pending task (FIFO) and mark it as running
    pub async fn dequeue(&self) -> Result<Option<Task>> {
        let path = self.db_path.clone();

        task::spawn_blocking(move || {
            let mut conn = Connection::open(&path)?;
            let tx = conn.transaction()?;

            // Find next task
            let task_row: Option<(String, String, String, String, String, String, i32, Option<String>)> = tx.query_row(
                "SELECT id, kind, payload, status, created_at, updated_at, attempts, last_error 
                 FROM tasks 
                 WHERE status = 'pending' 
                 ORDER BY created_at ASC 
                 LIMIT 1",
                [],
                |row| Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, 
                    row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?
                ))
            ).optional()?;

            if let Some((id, kind, payload, status, created_at, updated_at, attempts, last_error)) = task_row {
                // Update status
                let now = Utc::now().to_rfc3339();
                tx.execute(
                    "UPDATE tasks SET status = 'running', updated_at = ?1 WHERE id = ?2",
                    params![&now, &id],
                )?;
                tx.commit()?;

                Ok(Some(Task {
                    id,
                    kind,
                    payload,
                    status: TaskStatus::from(status),
                    created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
                    updated_at: DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&Utc),
                    attempts,
                    last_error,
                }))
            } else {
                Ok(None)
            }
        }).await?
    }

    /// Mark a task as completed
    pub async fn complete(&self, task_id: &str) -> Result<()> {
        let path = self.db_path.clone();
        let id = task_id.to_string();

        task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                params![&now, &id],
            )?;
            Ok::<_, anyhow::Error>(())
        }).await?
    }

    /// Mark a task as failed, optionally scheduling a retry
    pub async fn fail(&self, task_id: &str, error: &str, should_retry: bool) -> Result<()> {
        let path = self.db_path.clone();
        let id = task_id.to_string();
        let err_msg = error.to_string();

        task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let now = Utc::now().to_rfc3339();
            let new_status = if should_retry { "pending" } else { "failed" };
            
            conn.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2, attempts = attempts + 1, last_error = ?3 WHERE id = ?4",
                params![new_status, &now, &err_msg, &id],
            )?;
            Ok::<_, anyhow::Error>(())
        }).await?
    }

    /// Get task status
    pub async fn get_status(&self, task_id: &str) -> Result<Option<String>> {
        let path = self.db_path.clone();
        let id = task_id.to_string();

        task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let status: Option<String> = conn.query_row(
                "SELECT status FROM tasks WHERE id = ?1",
                params![&id],
                |row| row.get(0),
            ).optional()?;
            Ok::<_, anyhow::Error>(status)
        }).await?
    }

    /// Count tasks by status
    pub async fn count(&self, status: &str) -> Result<i64> {
        let path = self.db_path.clone();
        let status_str = status.to_string();

        task::spawn_blocking(move || {
            let conn = Connection::open(&path)?;
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE status = ?1",
                params![&status_str],
                |row| row.get(0),
            )?;
            Ok::<_, anyhow::Error>(count)
        }).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use serde_json::json;

    #[tokio::test]
    async fn test_queue_workflow() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let queue = TaskQueue::new(temp_file.path()).await?;

        // Enqueue
        let task_id = queue.enqueue("test_job", json!({"foo": "bar"})).await?;
        assert_eq!(queue.count("pending").await?, 1);

        // Dequeue
        let task = queue.dequeue().await?.expect("Should have task");
        assert_eq!(task.id, task_id);
        assert_eq!(task.kind, "test_job");
        assert_eq!(queue.count("running").await?, 1);

        // Complete
        queue.complete(&task_id).await?;
        assert_eq!(queue.count("completed").await?, 1);
        assert_eq!(queue.get_status(&task_id).await?.unwrap(), "completed");

        Ok(())
    }
}
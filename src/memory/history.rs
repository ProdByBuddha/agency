//! Global, append-only message history persistence
//! 
//! Provides a production-grade implementation of JSONL-based history
//! with advisory file locking and automatic size-based trimming.
//! Derived from codex-rs patterns.

use std::fs::File;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Result, Seek, SeekFrom, Write};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::fs as tokio_fs;
use tracing::debug;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use fs2::FileExt;

/// Filename that stores the message history
    const HISTORY_FILENAME: &str = "data/agency_history.jsonl";
/// When history exceeds the hard cap, trim it down to this fraction of `max_bytes`.
const HISTORY_SOFT_CAP_RATIO: f64 = 0.8;

const MAX_RETRIES: usize = 10;
const RETRY_SLEEP: Duration = Duration::from_millis(100);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub session_id: String,
    pub role: String,
    pub agent: Option<String>,
    pub ts: u64,
    pub text: String,
}

pub struct HistoryManager {
    path: PathBuf,
    max_bytes: Option<usize>,
}

impl HistoryManager {
    pub fn new(path: impl Into<PathBuf>, max_bytes: Option<usize>) -> Self {
        Self {
            path: path.into(),
            max_bytes,
        }
    }

    pub fn default_path() -> PathBuf {
        PathBuf::from(HISTORY_FILENAME)
    }

    /// Append a text entry to the history file.
    pub async fn append(&self, session_id: &str, role: &str, agent: Option<&str>, text: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio_fs::create_dir_all(parent).await?;
        }

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| std::io::Error::other(format!("system clock before Unix epoch: {e}")))?
            .as_secs();

        let entry = HistoryEntry {
            session_id: session_id.to_string(),
            role: role.to_string(),
            agent: agent.map(|s| s.to_string()),
            ts,
            text: text.to_string(),
        };

        let mut line = serde_json::to_string(&entry)
            .map_err(|e| std::io::Error::other(format!("failed to serialize history entry: {e}")))?;
        line.push('\n');

        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            options.append(true);
            options.mode(0o600);
        }

        let mut file = options.open(&self.path)?;
        
        #[cfg(unix)]
        self.ensure_owner_only_permissions(&file).await?;

        let max_bytes = self.max_bytes;
        let _path_clone = self.path.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            for _ in 0..MAX_RETRIES {
                #[cfg(unix)]
                match file.try_lock_exclusive() {
                    Ok(()) => {
                        file.seek(SeekFrom::End(0))?;
                        file.write_all(line.as_bytes())?;
                        file.flush()?;
                        Self::enforce_limit(&mut file, max_bytes)?;
                        let _ = file.unlock();
                        return Ok(());
                    }
                    Err(_) => {
                        std::thread::sleep(RETRY_SLEEP);
                    }
                }
                #[cfg(not(unix))] 
                {
                    // Basic fallback for non-unix if needed, but the user is on darwin
                    file.seek(SeekFrom::End(0))?;
                    file.write_all(line.as_bytes())?;
                    file.flush()?;
                    return Ok(());
                }
            }

            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "could not acquire exclusive lock on history file"
            ))
        })
        .await??;

        Ok(())
    }

    fn enforce_limit(file: &mut File, max_bytes: Option<usize>) -> Result<()> {
        let Some(max_bytes) = max_bytes else { return Ok(()); };
        if max_bytes == 0 { return Ok(()); }

        let mut current_len = file.metadata()?.len();
        if current_len <= max_bytes as u64 { return Ok(()); }

        debug!("Trimming history file (current size: {} bytes, limit: {} bytes)", current_len, max_bytes);

        let mut reader_file = file.try_clone()?;
        reader_file.seek(SeekFrom::Start(0))?;
        let mut buf_reader = BufReader::new(reader_file);
        let mut line_lengths = Vec::new();
        let mut line_buf = String::new();

        loop {
            line_buf.clear();
            let bytes = buf_reader.read_line(&mut line_buf)?;
            if bytes == 0 { break; }
            line_lengths.push(bytes as u64);
        }

        if line_lengths.is_empty() { return Ok(()); }

        let soft_cap = ((max_bytes as f64) * HISTORY_SOFT_CAP_RATIO) as u64;
        let mut drop_bytes = 0u64;
        let mut idx = 0usize;

        while current_len > soft_cap && idx < line_lengths.len() - 1 {
            current_len -= line_lengths[idx];
            drop_bytes += line_lengths[idx];
            idx += 1;
        }

        if drop_bytes == 0 { return Ok(()); }

        let mut reader = buf_reader.into_inner();
        reader.seek(SeekFrom::Start(drop_bytes))?;
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail)?;

        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&tail)?;
        file.flush()?;

        Ok(())
    }

    #[cfg(unix)]
    async fn ensure_owner_only_permissions(&self, file: &File) -> Result<()> {
        let metadata = file.metadata()?;
        let current_mode = metadata.permissions().mode() & 0o777;
        if current_mode != 0o600 {
            let mut perms = metadata.permissions();
            perms.set_mode(0o600);
            let perms_clone = perms.clone();
            let file_clone = file.try_clone()?;
            tokio::task::spawn_blocking(move || file_clone.set_permissions(perms_clone)).await??;
        }
        Ok(())
    }

    pub async fn load_recent(&self, n: usize) -> Result<Vec<HistoryEntry>> {
        if !self.path.exists() { return Ok(Vec::new()); }

        let content = tokio_fs::read_to_string(&self.path).await?;
        let entries: Vec<HistoryEntry> = content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(entries.into_iter().rev().take(n).rev().collect())
    }
}

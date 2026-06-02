//! SFTP file operations over an established SSH session (russh-sftp 2.3.0).
//!
//! The mobile file browser needs directory listing, stat, read/write, and basic
//! mutation (mkdir/remove/rename) against the remote cluster. On mobile there is
//! no Python sidecar, so these Tauri commands are the only path to the remote
//! filesystem — they drive `russh-sftp`'s high-level `SftpSession` over a fresh
//! SFTP subsystem channel opened on the session's russh `Handle`.
//!
//! Channel lifecycle (confirmed against the vendored russh-sftp 2.3.0 source):
//!   handle.channel_open_session()        -> russh `Channel<Msg>`
//!   channel.request_subsystem(true, "sftp")
//!   SftpSession::new(channel.into_stream()).await
//! A fresh SFTP session is opened per command (step 4); caching is a later
//! optional optimization (`state.rs` is unchanged).
//!
//! Contract:
//!   * every command returns `Result<_, String>` and NEVER panics — all errors
//!     (no session / dead session / transport / SFTP status) become an `Err`
//!     string the frontend renders.
//!   * a leading `~` is expanded to the SFTP server's default directory
//!     (`canonicalize(".")`) when cheap; if that probe fails the path is left
//!     as-is so the server can resolve it.
//!
//! Relevant russh-sftp 2.3.0 API (verified, not guessed):
//!   * `SftpSession::new(stream).await -> SftpResult<SftpSession>`
//!   * `read_dir(path).await -> SftpResult<ReadDir>` (Iterator of `DirEntry`)
//!       - `DirEntry::file_name() -> String`, `::path() -> String`,
//!         `::metadata() -> Metadata`
//!   * `metadata(path).await -> SftpResult<Metadata>` (`Metadata = FileAttributes`)
//!       - `FileAttributes { size: Option<u64>, .. }`, `::is_dir() -> bool`
//!   * `read(path).await -> SftpResult<Vec<u8>>`
//!   * `write(path, &[u8]).await -> SftpResult<()>`
//!   * `create_dir(path).await -> SftpResult<()>`
//!   * `remove_file(path).await -> SftpResult<()>`
//!   * `rename(old, new).await -> SftpResult<()>`
//!   * `canonicalize(path).await -> SftpResult<String>` (used for `~` expansion)

use russh_sftp::client::SftpSession;
use serde::Serialize;

use super::state::{SshSession, SshState};

/// A single remote filesystem entry surfaced to the file browser.
#[derive(Debug, Clone, Serialize)]
pub struct SftpEntry {
    /// Base name (no directory component).
    pub name: String,
    /// Full remote path (POSIX `/`-joined).
    pub path: String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
    /// File size in bytes (0 when the server omits it).
    pub size: u64,
}

/// Result of reading a (possibly large) text file.
#[derive(Debug, Clone, Serialize)]
pub struct SftpReadResult {
    /// UTF-8 (lossy) decoded contents, capped at `max_bytes` when provided.
    pub content: String,
    /// True when the file was longer than `max_bytes` and got cut off.
    pub truncated: bool,
}

/// Resolve a session by id, returning a stable `Err(String)` when it is missing
/// or has been marked dead. Centralized so every command shares the contract.
async fn resolve_session(
    state: &SshState,
    session_id: &str,
) -> Result<std::sync::Arc<SshSession>, String> {
    match state.get(session_id).await {
        Some(s) if s.is_alive() => Ok(s),
        Some(_) => Err(format!("SSH session {session_id} is no longer alive")),
        None => Err(format!("No such SSH session: {session_id}")),
    }
}

/// Open a fresh SFTP subsystem over the session's russh handle.
///
/// `channel_open_session` is `&self` on the russh write half, but we take the
/// session handle lock for the open to serialize with the auth/exec users of the
/// same handle. The returned `SftpSession` owns the channel stream and is
/// independent of the lock once built, so the guard is dropped here.
async fn open_sftp(session: &SshSession) -> Result<SftpSession, String> {
    let channel = {
        let handle = session.handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|e| format!("channel_open_session failed: {e}"))?
    };
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("request_subsystem(sftp) failed: {e}"))?;
    SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| format!("SFTP init failed: {e}"))
}

/// Expand a leading `~` to the SFTP server's default directory.
///
/// `~` -> default dir; `~/sub` -> default dir + `/sub`. The default dir is
/// probed via `canonicalize(".")`. If that probe fails (rare), the original
/// path is returned unchanged so the server can attempt its own resolution —
/// this is a best-effort convenience, never a hard error.
async fn expand_tilde(sftp: &SftpSession, path: &str) -> String {
    if path != "~" && !path.starts_with("~/") {
        return path.to_string();
    }
    let home = match sftp.canonicalize(".").await {
        Ok(h) => h,
        Err(_) => return path.to_string(),
    };
    let home = home.trim_end_matches('/');
    if path == "~" {
        return home.to_string();
    }
    // path starts with "~/": keep the rest after the tilde.
    format!("{}{}", home, &path[1..])
}

/// List the entries of a remote directory.
#[tauri::command]
pub async fn sftp_list(
    session_id: String,
    path: String,
    state: tauri::State<'_, SshState>,
) -> Result<Vec<SftpEntry>, String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let path = expand_tilde(&sftp, &path).await;

    let read_dir = sftp
        .read_dir(&path)
        .await
        .map_err(|e| format!("read_dir({path}) failed: {e}"))?;

    let mut entries: Vec<SftpEntry> = Vec::new();
    for entry in read_dir {
        let meta = entry.metadata();
        entries.push(SftpEntry {
            name: entry.file_name(),
            path: entry.path(),
            is_dir: meta.is_dir(),
            size: meta.size.unwrap_or(0),
        });
    }
    Ok(entries)
}

/// Stat a single remote path.
#[tauri::command]
pub async fn sftp_stat(
    session_id: String,
    path: String,
    state: tauri::State<'_, SshState>,
) -> Result<SftpEntry, String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let path = expand_tilde(&sftp, &path).await;

    let meta = sftp
        .metadata(&path)
        .await
        .map_err(|e| format!("stat({path}) failed: {e}"))?;

    // Derive the base name from the (already tilde-expanded) path.
    let name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(&path)
        .to_string();

    Ok(SftpEntry {
        name,
        path,
        is_dir: meta.is_dir(),
        size: meta.size.unwrap_or(0),
    })
}

/// Read a remote file as UTF-8 (lossy) text, optionally capped at `max_bytes`.
#[tauri::command]
pub async fn sftp_read(
    session_id: String,
    path: String,
    max_bytes: Option<usize>,
    state: tauri::State<'_, SshState>,
) -> Result<SftpReadResult, String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let path = expand_tilde(&sftp, &path).await;

    let bytes = sftp
        .read(&path)
        .await
        .map_err(|e| format!("read({path}) failed: {e}"))?;

    let (slice, truncated) = match max_bytes {
        Some(cap) if bytes.len() > cap => (&bytes[..cap], true),
        _ => (&bytes[..], false),
    };

    Ok(SftpReadResult {
        content: String::from_utf8_lossy(slice).into_owned(),
        truncated,
    })
}

/// Read a remote file as raw bytes (for binary downloads / non-text files).
#[tauri::command]
pub async fn sftp_read_bytes(
    session_id: String,
    path: String,
    state: tauri::State<'_, SshState>,
) -> Result<Vec<u8>, String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let path = expand_tilde(&sftp, &path).await;

    sftp.read(&path)
        .await
        .map_err(|e| format!("read_bytes({path}) failed: {e}"))
}

/// Write (create/truncate) a remote text file.
#[tauri::command]
pub async fn sftp_write(
    session_id: String,
    path: String,
    content: String,
    state: tauri::State<'_, SshState>,
) -> Result<(), String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let path = expand_tilde(&sftp, &path).await;

    sftp.write(&path, content.as_bytes())
        .await
        .map_err(|e| format!("write({path}) failed: {e}"))
}

/// Create a remote directory.
#[tauri::command]
pub async fn sftp_mkdir(
    session_id: String,
    path: String,
    state: tauri::State<'_, SshState>,
) -> Result<(), String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let path = expand_tilde(&sftp, &path).await;

    sftp.create_dir(&path)
        .await
        .map_err(|e| format!("mkdir({path}) failed: {e}"))
}

/// Remove a remote file.
#[tauri::command]
pub async fn sftp_remove(
    session_id: String,
    path: String,
    state: tauri::State<'_, SshState>,
) -> Result<(), String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let path = expand_tilde(&sftp, &path).await;

    sftp.remove_file(&path)
        .await
        .map_err(|e| format!("remove({path}) failed: {e}"))
}

/// Rename / move a remote path.
#[tauri::command]
pub async fn sftp_rename(
    session_id: String,
    from: String,
    to: String,
    state: tauri::State<'_, SshState>,
) -> Result<(), String> {
    let session = resolve_session(&state, &session_id).await?;
    let sftp = open_sftp(&session).await?;
    let from = expand_tilde(&sftp, &from).await;
    let to = expand_tilde(&sftp, &to).await;

    sftp.rename(&from, &to)
        .await
        .map_err(|e| format!("rename({from} -> {to}) failed: {e}"))
}

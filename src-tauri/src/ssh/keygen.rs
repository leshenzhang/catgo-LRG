//! SSH-key passwordless-login setup (keypair generation, public-key install,
//! and private-key at-rest protection).
//!
//! Flow (driven by the mobile `KeySetup.svelte` UI after a successful Duo
//! login):
//!   1. `ssh_keygen`         — generate a fresh ed25519 keypair ON THE DEVICE and
//!                             return both halves (OpenSSH wire forms) to the UI.
//!   2. `ssh_install_pubkey` — over the LIVE session, idempotently append the
//!                             PUBLIC key to the cluster's `~/.ssh/authorized_keys`
//!                             (so future connects can use publickey auth).
//!   3. `ssh_key_store`      — wrap the PRIVATE key at rest and persist it under
//!                             the app data dir; `ssh_key_load` unwraps it.
//!
//! At-rest protection — IMPORTANT (implemented vs. intended):
//!   * IMPLEMENTED HERE (works on every target, no platform plugin needed): an
//!     AES-256-GCM envelope. The data-encryption key (DEK) is generated with a
//!     CSPRNG (`getrandom`) and stored alongside the ciphertext under the app
//!     data dir (0600 on Unix). This is a software fallback: it protects the key
//!     from casual at-rest disclosure but the DEK is NOT hardware-bound.
//!   * INTENDED HARDENING (documented, NOT wired here): on Android the DEK should
//!     be generated/held inside the AndroidKeyStore (hardware-backed,
//!     non-exportable) and used to wrap the private key — see
//!     `deploy/android/README.md`. That requires a small Kotlin Tauri plugin; the
//!     Rust surface below is deliberately shaped so swapping the DEK source for a
//!     Keystore-held key is a localized change (`wrap_dek` / `unwrap_dek`).
//!
//! This module is NOT `#[cfg]`-gated (like the rest of `ssh`): it compiles and is
//! validated on desktop too. The commands are registered in BOTH invoke handlers
//! in `lib.rs`.

use std::io::Write as _;
use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use russh::keys::ssh_key::{
    private::Ed25519Keypair,
    rand_core::{CryptoRng, RngCore},
    LineEnding, PrivateKey,
};

use super::exec::ExecResult;
use super::state::SshState;

/// A `CryptoRngCore` adapter backed by the OS CSPRNG (`getrandom`).
///
/// `ssh-key`'s `rand_core` reexport is built WITHOUT its `getrandom` feature, so
/// `rand_core::OsRng` is unavailable. We provide a tiny zero-sized adapter that
/// satisfies `RngCore + CryptoRng` (the `CryptoRngCore` bound on
/// `Ed25519Keypair::random`) by pulling every byte straight from `getrandom`.
struct OsCsprng;

impl RngCore for OsCsprng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).expect("OS CSPRNG failure");
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        getrandom::getrandom(&mut b).expect("OS CSPRNG failure");
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        getrandom::getrandom(dest).expect("OS CSPRNG failure");
    }
    fn try_fill_bytes(
        &mut self,
        dest: &mut [u8],
    ) -> Result<(), russh::keys::ssh_key::rand_core::Error> {
        // `getrandom` failing is fatal for keygen; surface it as a panic via
        // `fill_bytes` rather than fabricating a rand_core error code (whose
        // constructor is private in this rand_core version).
        self.fill_bytes(dest);
        Ok(())
    }
}

// SAFETY MARKER: getrandom is a cryptographically-secure OS RNG.
impl CryptoRng for OsCsprng {}

/// A freshly-generated keypair in OpenSSH wire form.
#[derive(Debug, Clone, Serialize)]
pub struct KeyPair {
    /// `ssh-ed25519 AAAA... comment` — append this to `authorized_keys`.
    pub public_openssh: String,
    /// OpenSSH PEM private key (`-----BEGIN OPENSSH PRIVATE KEY----- ...`),
    /// UNENCRYPTED (the device protects it at rest via `ssh_key_store`).
    pub private_openssh: String,
}

/// Generate a fresh ed25519 SSH keypair on the device.
///
/// Uses `ssh-key`'s own ed25519 helper seeded from the OS CSPRNG (`OsRng`), so no
/// extra crypto dependency is introduced. NEVER throws across the Tauri boundary
/// for an expected reason — only a genuine internal serialization fault yields
/// `Err`.
#[tauri::command]
pub async fn ssh_keygen() -> Result<KeyPair, String> {
    // ed25519: small, fast, universally accepted by modern OpenSSH servers
    // (Expanse included). Seeded from the OS CSPRNG.
    let keypair = Ed25519Keypair::random(&mut OsCsprng);
    let mut private = PrivateKey::from(keypair);
    // A stable comment makes the authorized_keys entry recognizable on the
    // cluster (and lets the user prune it later).
    private.set_comment("catgo-mobile");

    let private_openssh = private
        .to_openssh(LineEnding::LF)
        .map_err(|e| format!("could not encode private key: {e}"))?
        .to_string();

    let public_openssh = private
        .public_key()
        .to_openssh()
        .map_err(|e| format!("could not encode public key: {e}"))?;

    Ok(KeyPair {
        public_openssh,
        private_openssh,
    })
}

/// POSIX single-quote escaping (same contract as `exec::shlex_quote`, kept local
/// so this module has no cross-module private dependency).
fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Install a PUBLIC key into the live session's `~/.ssh/authorized_keys`.
///
/// Runs an idempotent shell snippet that:
///   * creates `~/.ssh` with mode 700 if missing,
///   * appends the key to `authorized_keys` (mode 600) ONLY if an identical line
///     is not already present (`grep -qF`),
/// so repeated runs never duplicate the entry. Returns the remote command result
/// (`code == 0` => installed/already-present). NEVER throws for a remote failure:
/// surfaces `ExecResult { code: -1, stderr }` like `ssh_exec`.
#[tauri::command]
pub async fn ssh_install_pubkey(
    session_id: String,
    public_openssh: String,
    state: tauri::State<'_, SshState>,
) -> Result<ExecResult, String> {
    let session = match state.get(&session_id).await {
        Some(s) => s,
        None => {
            return Ok(ExecResult::error(format!(
                "No such SSH session: {session_id}"
            )));
        }
    };
    if !session.is_alive() {
        return Ok(ExecResult::error(format!(
            "SSH session {session_id} is no longer alive"
        )));
    }

    // Reject obviously-malformed input early (defence-in-depth; the key still
    // gets single-quoted before it touches the shell).
    let key = public_openssh.trim();
    if !key.starts_with("ssh-") && !key.starts_with("ecdsa-") {
        return Ok(ExecResult::error(
            "refusing to install: not an OpenSSH public key".to_string(),
        ));
    }
    let quoted = shell_single_quote(key);

    // Idempotent install. `grep -qF -- "$KEY"` guards the append. `umask`/explicit
    // chmod harden the perms (OpenSSH refuses world-readable key material).
    let snippet = format!(
        r#"set -e
mkdir -p ~/.ssh
chmod 700 ~/.ssh
touch ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
KEY={quoted}
if grep -qF -- "$KEY" ~/.ssh/authorized_keys; then
  echo "already-present"
else
  printf '%s\n' "$KEY" >> ~/.ssh/authorized_keys
  echo "installed"
fi"#,
    );

    let login_cmd = format!("bash -l -c {}", shell_single_quote(&snippet));

    let handle = session.handle.lock().await;
    let channel = match handle.channel_open_session().await {
        Ok(c) => c,
        Err(e) => {
            session.mark_dead();
            return Ok(ExecResult::error(format!(
                "channel_open_session failed: {e}"
            )));
        }
    };
    if let Err(e) = channel.exec(true, login_cmd.into_bytes()).await {
        return Ok(ExecResult::error(format!("exec failed: {e}")));
    }

    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let mut code: Option<i32> = None;
    let mut channel = channel;
    while let Some(msg) = channel.wait().await {
        match msg {
            russh::ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            russh::ChannelMsg::ExtendedData { data, ext } if ext == 1 => {
                stderr.extend_from_slice(&data)
            }
            russh::ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status as i32),
            _ => {}
        }
    }

    Ok(ExecResult {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        code: code.unwrap_or(0),
    })
}

// ─────────────────────────── private-key at-rest storage ───────────────────────────

/// The on-disk envelope persisted under the app data dir.
///
/// `dek` is the data-encryption key. In THIS (software) implementation it is a
/// random 32-byte key stored next to the ciphertext. In the INTENDED hardening
/// (`deploy/android/README.md`) the DEK is generated/held by the AndroidKeyStore
/// and only its (Keystore-wrapped) handle would be stored here.
#[derive(Debug, Serialize, Deserialize)]
struct KeyEnvelope {
    /// Envelope format version (bump if the wrapping scheme changes).
    v: u8,
    /// 12-byte AES-GCM nonce (base64).
    nonce_b64: String,
    /// AES-256-GCM ciphertext of the OpenSSH private key (base64).
    ct_b64: String,
    /// 32-byte data-encryption key (base64). SOFTWARE FALLBACK ONLY — see the
    /// module docs / Android README for the hardware-backed plan.
    dek_b64: String,
}

/// Minimal, dependency-free base64 (standard alphabet, padded). Kept local so the
/// storage path adds no new transitive crate just for encoding.
mod b64 {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(A[((n >> 18) & 63) as usize] as char);
            out.push(A[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                A[((n >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                A[(n & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        let mut rev = [255u8; 256];
        for (i, &c) in A.iter().enumerate() {
            rev[c as usize] = i as u8;
        }
        let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=' && !c.is_ascii_whitespace()).collect();
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for chunk in bytes.chunks(4) {
            let mut n = 0u32;
            let mut valid = 0;
            for (i, &c) in chunk.iter().enumerate() {
                let v = rev[c as usize];
                if v == 255 {
                    return Err("invalid base64".to_string());
                }
                n |= u32::from(v) << (18 - 6 * i);
                valid += 1;
            }
            if valid >= 2 {
                out.push((n >> 16) as u8);
            }
            if valid >= 3 {
                out.push((n >> 8) as u8);
            }
            if valid >= 4 {
                out.push(n as u8);
            }
        }
        Ok(out)
    }
}

/// Resolve the per-app stored-key path for a given endpoint key.
///
/// `endpoint_key` is an opaque, caller-supplied identity (e.g.
/// `host:port:username`); it is sanitized to a filesystem-safe basename so two
/// clusters never collide.
fn stored_key_path(app: &tauri::AppHandle, endpoint_key: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data dir: {e}"))?
        .join("ssh_keys");
    let safe: String = endpoint_key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    Ok(dir.join(format!("{safe}.json")))
}

/// SOFTWARE-FALLBACK DEK source: a fresh random 32-byte key.
///
/// HARDENING SEAM: on Android this is where a Keystore-held / hardware-backed key
/// would be substituted (see module docs). Returning the DEK here keeps the call
/// sites identical when that swap happens.
fn wrap_dek() -> Result<[u8; 32], String> {
    let mut dek = [0u8; 32];
    getrandom::getrandom(&mut dek).map_err(|e| format!("CSPRNG failure: {e}"))?;
    Ok(dek)
}

/// Persist a private key, AES-256-GCM-wrapped, under the app data dir.
///
/// `endpoint_key` identifies the cluster (e.g. `host:port:username`) so the key
/// can be looked up at the next connect. Overwrites any existing entry.
#[tauri::command]
pub async fn ssh_key_store(
    app: tauri::AppHandle,
    endpoint_key: String,
    private_openssh: String,
) -> Result<(), String> {
    let dek = wrap_dek()?;
    let cipher = Aes256Gcm::new_from_slice(&dek)
        .map_err(|e| format!("cipher init failed: {e}"))?;

    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| format!("CSPRNG failure: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ct = cipher
        .encrypt(nonce, private_openssh.as_bytes())
        .map_err(|e| format!("encryption failed: {e}"))?;

    let envelope = KeyEnvelope {
        v: 1,
        nonce_b64: b64::encode(&nonce_bytes),
        ct_b64: b64::encode(&ct),
        dek_b64: b64::encode(&dek),
    };
    let json = serde_json::to_string(&envelope).map_err(|e| format!("serialize failed: {e}"))?;

    let path = stored_key_path(&app, &endpoint_key)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    // Write 0600 from the start where possible.
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path).map_err(|e| format!("open failed: {e}"))?;
    f.write_all(json.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;
    Ok(())
}

/// Load + unwrap a previously-stored private key, or `None` if no key is stored
/// for `endpoint_key`.
#[tauri::command]
pub async fn ssh_key_load(
    app: tauri::AppHandle,
    endpoint_key: String,
) -> Result<Option<String>, String> {
    let path = stored_key_path(&app, &endpoint_key)?;
    let json = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read failed: {e}")),
    };

    let envelope: KeyEnvelope =
        serde_json::from_str(&json).map_err(|e| format!("corrupt key envelope: {e}"))?;
    let dek = b64::decode(&envelope.dek_b64)?;
    let nonce_bytes = b64::decode(&envelope.nonce_b64)?;
    let ct = b64::decode(&envelope.ct_b64)?;

    let cipher =
        Aes256Gcm::new_from_slice(&dek).map_err(|e| format!("cipher init failed: {e}"))?;
    let pt = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ct.as_ref())
        .map_err(|e| format!("decryption failed (tampered or wrong key): {e}"))?;
    let key = String::from_utf8(pt).map_err(|e| format!("decoded key not UTF-8: {e}"))?;
    Ok(Some(key))
}

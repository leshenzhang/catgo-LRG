/**
 * Live SSH session registry — ControlMaster-style connection reuse.
 *
 * Maps a cluster endpoint (`host:port:user`) to the id of a russh session that
 * is still alive on the Rust side. When the user leaves a session (⏏) we keep
 * the connection alive and remember it here; the next connect to the same
 * endpoint reuses it (after a liveness check) instead of re-authenticating — so
 * no second OTP. The russh connection is multiplexed (one TCP + auth, many
 * channels for exec/pty/sftp), exactly the ControlMaster benefit.
 *
 * Module-level state survives component remounts within the app process (it is
 * NOT persisted — a fresh app launch has no live sessions, which is correct
 * since SSH connections cannot outlive the process).
 */

import { transport } from '$lib/api/transport'

/** endpoint (`host:port:user`) -> live session id. */
const live = new Map<string, string>()

export function endpointKey(host: string, port: number, username: string): string {
  return `${host.trim()}:${port}:${username.trim()}`
}

/** Remember a freshly-authenticated session for reuse. */
export function rememberSession(key: string, sessionId: string): void {
  live.set(key, sessionId)
}

/** Forget an endpoint's session (e.g. after a confirmed-dead reuse attempt). */
export function forgetSession(key: string): void {
  live.delete(key)
}

/**
 * Return a still-alive session id for this endpoint, or null. Verifies liveness
 * with a cheap `true` exec; a failure drops the stale entry so the caller falls
 * back to a fresh connect.
 */
export async function reuseSession(key: string): Promise<string | null> {
  const id = live.get(key)
  if (!id) return null
  try {
    const r = await transport.exec(id, `true`, 6000)
    if (r.code === 0) return id
  } catch {
    /* dead / unreachable */
  }
  live.delete(key)
  return null
}

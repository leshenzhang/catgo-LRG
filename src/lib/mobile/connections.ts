/**
 * Saved cluster connections for the mobile UI.
 *
 * A small localStorage-backed list so the user can reconnect to a cluster with
 * one tap instead of retyping host / port / username / method every time.
 *
 * SECURITY: this NEVER stores secrets — no passwords, passphrases, or OTP
 * answers. Only the non-secret connection descriptor is kept. (Private SSH keys
 * for passwordless login are handled separately, wrapped by the platform
 * keystore — not here.)
 */

import type { HpcAuthMethod } from '$lib/api/transport'

/** A non-secret saved connection descriptor. */
export interface SavedConnection {
  /** Stable identity = `${host}:${port}:${username}:${method}`. */
  id: string
  host: string
  port: number
  username: string
  method: HpcAuthMethod
  /** Private-key path (publickey method only); never the key material itself. */
  keyPath?: string
  /** Epoch ms of the last successful connect — used for most-recent ordering. */
  lastUsed: number
}

const STORAGE_KEY = `catgo_mobile_connections`
/** Legacy single-entry key (pre-list); migrated into the list on first load. */
const LEGACY_KEY = `catgo_mobile_connect`

/** Stable id from the connection's identifying tuple. */
export function connectionId(
  host: string,
  port: number,
  username: string,
  method: HpcAuthMethod,
): string {
  return `${host}:${port}:${username}:${method}`
}

/** Human label for a saved connection (e.g. `gliu3@login.expanse.sdsc.edu`). */
export function connectionLabel(c: SavedConnection): string {
  return c.port === 22 ? `${c.username}@${c.host}` : `${c.username}@${c.host}:${c.port}`
}

function readRaw(): SavedConnection[] {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as SavedConnection[]
      if (Array.isArray(parsed)) return parsed.filter((c) => c && c.host && c.username)
    }
  } catch {
    // Corrupt / unavailable storage — fall through to legacy + empty.
  }
  return []
}

function migrateLegacy(list: SavedConnection[]): SavedConnection[] {
  if (list.length > 0) return list
  try {
    const raw = globalThis.localStorage?.getItem(LEGACY_KEY)
    if (!raw) return list
    const old = JSON.parse(raw) as {
      host?: string
      port?: number
      username?: string
      method?: HpcAuthMethod
      key_path?: string
    }
    if (old.host && old.username) {
      const port = typeof old.port === `number` ? old.port : 22
      const method = old.method ?? `password`
      return [
        {
          id: connectionId(old.host, port, old.username, method),
          host: old.host,
          port,
          username: old.username,
          method,
          keyPath: old.key_path || undefined,
          lastUsed: 0,
        },
      ]
    }
  } catch {
    // Ignore a corrupt legacy entry.
  }
  return list
}

/** Load saved connections, most-recently-used first. */
export function loadConnections(): SavedConnection[] {
  const list = migrateLegacy(readRaw())
  return [...list].sort((a, b) => b.lastUsed - a.lastUsed)
}

function write(list: SavedConnection[]): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(list))
  } catch {
    // Storage unavailable — non-fatal; the in-memory list still works this run.
  }
}

/**
 * Insert or update a connection (deduped by identity) and stamp `lastUsed`.
 * Returns the new most-recent-first list.
 */
export function upsertConnection(
  fields: { host: string; port: number; username: string; method: HpcAuthMethod; keyPath?: string },
  now: number,
): SavedConnection[] {
  const id = connectionId(fields.host, fields.port, fields.username, fields.method)
  const others = readRaw().filter((c) => c.id !== id)
  const entry: SavedConnection = {
    id,
    host: fields.host,
    port: fields.port,
    username: fields.username,
    method: fields.method,
    keyPath: fields.keyPath || undefined,
    lastUsed: now,
  }
  const next = [entry, ...others].sort((a, b) => b.lastUsed - a.lastUsed)
  write(next)
  return next
}

/** Remove a connection by id; returns the new most-recent-first list. */
export function removeConnection(id: string): SavedConnection[] {
  const next = readRaw()
    .filter((c) => c.id !== id)
    .sort((a, b) => b.lastUsed - a.lastUsed)
  write(next)
  return next
}

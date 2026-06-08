/**
 * Desktop-only helper for remembering HPC passwords in the Tauri app.
 *
 * The Python backend still receives the password only for the live connection
 * attempt. Saved HPC profiles remain non-secret JSON; the password is stored
 * separately through the existing Tauri wrapped-at-rest store.
 */

function isTauri(): boolean {
  return (
    typeof window !== `undefined` &&
    (`__TAURI__` in window || `__TAURI_INTERNALS__` in window)
  )
}

export function hpcPasswordKey(host: string, port: number, username: string): string {
  return `hpc-pw:${host.trim()}:${port}:${username.trim()}`
}

export async function loadHpcPassword(
  host: string,
  port: number,
  username: string,
): Promise<string | null> {
  if (!isTauri() || !host.trim() || !username.trim()) return null
  try {
    const { invoke } = await import(`@tauri-apps/api/core`)
    return await invoke<string | null>(`ssh_key_load`, {
      endpointKey: hpcPasswordKey(host, port, username),
    })
  } catch {
    return null
  }
}

export async function storeHpcPassword(
  host: string,
  port: number,
  username: string,
  password: string,
): Promise<boolean> {
  if (!isTauri() || !host.trim() || !username.trim() || !password) return false
  try {
    const { invoke } = await import(`@tauri-apps/api/core`)
    await invoke<void>(`ssh_key_store`, {
      endpointKey: hpcPasswordKey(host, port, username),
      privateOpenssh: password,
    })
    return true
  } catch {
    return false
  }
}

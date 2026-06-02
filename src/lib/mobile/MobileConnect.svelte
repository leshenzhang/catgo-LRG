<!--
  MobileConnect.svelte — SSH/HPC connect form for the mobile (tauri-ssh)
  transport.

  Collects host / port / username / auth-method (+ password or key path), then
  drives `transport.connect(...)`. If the server requires keyboard-interactive
  2FA (`needsOtp`), it shows {@link OtpDialog} and loops
  `transport.submitOtp(pendingId, responses)`:

    - a `Success` reply ends the loop and emits `session_id` to the parent;
    - an `InfoRequest` reply (another `needsOtp` with a fresh `pendingId` /
      `prompts`) re-shows the dialog for the next round (true multi-round 2FA).

  Persists the last host / username / method / port in localStorage as a
  convenience. NEVER persists passwords, passphrases, or OTP answers.
-->
<script lang="ts">
  import { transport, type HpcAuthMethod, type OtpPrompt } from '$lib/api/transport'
  import OtpDialog from './OtpDialog.svelte'
  import {
    loadConnections,
    upsertConnection,
    removeConnection,
    connectionLabel,
    type SavedConnection,
  } from './connections'

  interface Props {
    /** Emitted with the live session id once authentication completes. */
    on_connected?: (session_id: string) => void
  }

  let { on_connected }: Props = $props()

  // ─── Saved (non-secret) connections ───
  let saved = $state<SavedConnection[]>([])

  // ─── Form state ───
  let host = $state(``)
  let port = $state(22)
  let username = $state(``)
  let method = $state<HpcAuthMethod>(`password`)
  let password = $state(``)
  let key_path = $state(``)
  let passphrase = $state(``)

  // ─── Flow state ───
  let connecting = $state(false)
  let error_msg = $state(``)

  // ─── OTP round state ───
  let otp_visible = $state(false)
  let otp_busy = $state(false)
  let otp_pending_id = $state(``)
  let otp_prompts = $state<OtpPrompt[]>([])
  let otp_instructions = $state(``)

  // Load the saved-connection list and prefill the form from the most recent.
  // NOTE: read the fresh list into a LOCAL — never read `saved` back inside this
  // effect. `loadConnections()` returns a new array each call, so writing AND
  // reading `saved` here would make the effect depend on a value it just changed
  // → infinite re-run (svelte effect_update_depth_exceeded). Only `host` is a
  // tracked read, and the `!host` guard makes it converge after the first set.
  $effect(() => {
    const list = loadConnections()
    saved = list
    const recent = list[0]
    if (recent && !host) {
      host = recent.host
      port = recent.port
      username = recent.username
      method = recent.method
      if (recent.keyPath) key_path = recent.keyPath
    }
  })

  /** Fill the form from a saved connection (tap-to-reconnect). */
  function pick_saved(c: SavedConnection): void {
    host = c.host
    port = c.port
    username = c.username
    method = c.method
    key_path = c.keyPath ?? ``
    error_msg = ``
  }

  /** Delete a saved connection (does not touch any stored key material). */
  function delete_saved(id: string, e: Event): void {
    e.stopPropagation()
    saved = removeConnection(id)
  }

  function persist_non_secrets(): void {
    saved = upsertConnection(
      { host, port, username, method, keyPath: key_path },
      Date.now(),
    )
  }

  /** Apply a connect / submitOtp result: succeed, advance OTP round, or error. */
  function apply_result(r: {
    connected: boolean
    sessionId: string
    needsOtp: boolean
    message: string
    pendingId: string
    prompts: OtpPrompt[]
    instructions: string
  }): void {
    if (r.needsOtp) {
      // Show (or re-show) the dialog with this round's prompts.
      otp_pending_id = r.pendingId
      otp_prompts = r.prompts
      otp_instructions = r.instructions
      otp_visible = true
      otp_busy = false
      return
    }
    if (r.connected && r.sessionId) {
      otp_visible = false
      otp_busy = false
      persist_non_secrets()
      on_connected?.(r.sessionId)
      return
    }
    // Not connected and no OTP round => authentication failed / refused.
    otp_visible = false
    otp_busy = false
    error_msg = r.message || `Connection failed.`
  }

  async function connect(): Promise<void> {
    if (connecting) return
    error_msg = ``
    connecting = true
    try {
      const r = await transport.connect({
        host: host.trim(),
        port,
        username: username.trim(),
        method,
        password: method === `password` ? password : undefined,
        keyPath: method === `publickey` ? key_path.trim() || undefined : undefined,
        passphrase: method === `publickey` ? passphrase || undefined : undefined,
      })
      apply_result(r)
    } catch (e: unknown) {
      error_msg = e instanceof Error ? e.message : String(e)
      otp_visible = false
    } finally {
      connecting = false
      // Drop the in-memory password as soon as the round-trip completes; OTP
      // rounds (if any) no longer need it.
      password = ``
      passphrase = ``
    }
  }

  /** One OTP round: submit answers, then loop on a follow-up InfoRequest. */
  async function submit_otp(responses: string[]): Promise<void> {
    otp_busy = true
    error_msg = ``
    try {
      const r = await transport.submitOtp(otp_pending_id, responses)
      apply_result(r)
    } catch (e: unknown) {
      error_msg = e instanceof Error ? e.message : String(e)
      otp_visible = false
      otp_busy = false
    }
  }

  function cancel_otp(): void {
    otp_visible = false
    otp_busy = false
    otp_pending_id = ``
    otp_prompts = []
    otp_instructions = ``
    error_msg = `Authentication cancelled.`
  }

  const can_submit = $derived(
    host.trim().length > 0 &&
      username.trim().length > 0 &&
      !connecting,
  )
</script>

<div class="connect-wrap">
  <div class="connect-card">
    <div class="connect-title">Connect to cluster</div>

    {#if saved.length > 0}
      <div class="saved-list">
        <span class="saved-label">Saved</span>
        {#each saved as c (c.id)}
          <div
            class="saved-row"
            role="button"
            tabindex="0"
            onclick={() => pick_saved(c)}
            onkeydown={(e) => {
              if (e.key === `Enter` || e.key === ` `) {
                e.preventDefault()
                pick_saved(c)
              }
            }}
          >
            <span class="saved-main">{connectionLabel(c)}</span>
            <span class="saved-method">{c.method}</span>
            <button
              type="button"
              class="saved-del"
              aria-label="Remove saved connection"
              onclick={(e) => delete_saved(c.id, e)}
            >
              ✕
            </button>
          </div>
        {/each}
      </div>
    {/if}

    <form
      class="connect-form"
      onsubmit={(e) => {
        e.preventDefault()
        if (can_submit) connect()
      }}
    >
      <label class="field host-field">
        <span>Host</span>
        <input
          type="text"
          inputmode="url"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          placeholder="login.cluster.edu"
          bind:value={host}
        />
      </label>

      <label class="field port-field">
        <span>Port</span>
        <input type="number" min="1" max="65535" bind:value={port} />
      </label>

      <label class="field user-field">
        <span>Username</span>
        <input
          type="text"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          bind:value={username}
        />
      </label>

      <label class="field method-field">
        <span>Auth method</span>
        <select bind:value={method}>
          <option value="password">Password</option>
          <option value="publickey">Public key</option>
          <option value="keyboard-interactive">Keyboard-interactive</option>
        </select>
      </label>

      {#if method === `password`}
        <label class="field">
          <span>Password</span>
          <input
            type="password"
            autocomplete="current-password"
            bind:value={password}
          />
        </label>
      {:else if method === `publickey`}
        <label class="field">
          <span>Private key path</span>
          <input
            type="text"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            placeholder="~/.ssh/id_ed25519"
            bind:value={key_path}
          />
        </label>
        <label class="field">
          <span>Passphrase (optional)</span>
          <input type="password" autocomplete="off" bind:value={passphrase} />
        </label>
      {:else}
        <div class="method-hint">
          You'll be prompted for any codes after connecting.
        </div>
      {/if}

      {#if error_msg}
        <div class="connect-error" role="alert">{error_msg}</div>
      {/if}

      <button type="submit" class="connect-btn" disabled={!can_submit}>
        {connecting ? `Connecting…` : `Connect`}
      </button>
    </form>
  </div>
</div>

{#if otp_visible}
  <OtpDialog
    prompts={otp_prompts}
    instructions={otp_instructions}
    busy={otp_busy}
    on_submit={submit_otp}
    on_cancel={cancel_otp}
  />
{/if}

<style>
  .connect-wrap {
    display: flex;
    align-items: flex-start;
    justify-content: center;
    width: 100%;
    height: 100%;
    padding: 16px;
    padding-top: max(16px, env(safe-area-inset-top));
    overflow-y: auto;
    background: var(--page-bg, #0e1117);
    box-sizing: border-box;
  }
  .connect-card {
    width: 100%;
    max-width: 480px;
    background: var(--surface-bg, #1a1a2e);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 12px;
    padding: 20px;
  }
  .connect-title {
    font-size: 1.15em;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
    margin-bottom: 16px;
  }
  .saved-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 18px;
    padding-bottom: 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .saved-label {
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
  .saved-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 48px;
    padding: 8px 12px;
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 8px;
    cursor: pointer;
  }
  .saved-row:hover,
  .saved-row:focus-visible {
    border-color: var(--accent-color, #3b82f6);
    outline: none;
  }
  .saved-main {
    flex: 1;
    min-width: 0;
    font-size: 15px;
    color: var(--text-color, #e0e0e0);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .saved-method {
    flex-shrink: 0;
    font-size: 0.72em;
    color: var(--text-color-muted, #94a3b8);
    background: rgba(255, 255, 255, 0.06);
    padding: 2px 8px;
    border-radius: 999px;
  }
  .saved-del {
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    font-size: 14px;
    line-height: 1;
    color: var(--text-color-muted, #94a3b8);
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
  }
  .saved-del:hover {
    color: #ff6b6b;
    background: rgba(255, 107, 107, 0.1);
  }
  .connect-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field > span {
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
  .field input,
  .field select {
    width: 100%;
    padding: 10px 12px;
    font-size: 16px; /* >=16px stops iOS zoom-on-focus. */
    color: var(--text-color, #e0e0e0);
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 8px;
    outline: none;
    box-sizing: border-box;
  }
  .field input:focus,
  .field select:focus {
    border-color: var(--accent-color, #3b82f6);
  }
  .method-hint {
    font-size: 0.82em;
    color: var(--text-color-muted, #94a3b8);
  }
  .connect-error {
    font-size: 0.85em;
    color: #ff6b6b;
    background: rgba(255, 107, 107, 0.1);
    border: 1px solid rgba(255, 107, 107, 0.3);
    border-radius: 8px;
    padding: 8px 10px;
  }
  .connect-btn {
    min-height: 48px;
    margin-top: 4px;
    font-size: 16px;
    font-weight: 600;
    color: #fff;
    background: var(--accent-color, #0a84ff);
    border: 1px solid var(--accent-color, #0a84ff);
    border-radius: 8px;
    cursor: pointer;
  }
  .connect-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>

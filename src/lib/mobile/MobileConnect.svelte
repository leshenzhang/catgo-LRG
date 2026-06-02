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
  let label = $state(``)
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

  // ─── Saved-password / OTP-only reconnect ───
  // A password loaded from the encrypted store for the picked connection; when a
  // password prompt arrives it is auto-answered so the user only types the OTP.
  let auto_password = $state(``)
  // Whether THIS connect used a saved password (so we don't re-offer to save it).
  let used_saved_pw = false
  // The password actually used this connect (form value or a keyboard-interactive
  // response), captured so we can offer to persist it after success.
  let captured_password = ``
  // Post-success "save password?" prompt, parked until the user decides so we
  // stay mounted (calling on_connected swaps us out).
  let save_prompt_visible = $state(false)
  let pending_session = ``
  let pending_pw = ``

  /** Per-endpoint key for the encrypted password store (no method in the key —
   * the password is the same regardless of the method tried). */
  function endpoint_pw_key(): string {
    return `pw:${host.trim()}:${port}:${username.trim()}`
  }

  /** A prompt that asks for the account PASSWORD (not the 2FA passcode). */
  function is_password_prompt(p: OtpPrompt): boolean {
    return (
      !p.echo &&
      /pass\s*word/i.test(p.prompt) &&
      !/duo|passcode|one.?time|\botp\b|verification|token/i.test(p.prompt)
    )
  }

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
      label = recent.label ?? ``
      host = recent.host
      port = recent.port
      username = recent.username
      method = recent.method
      if (recent.keyPath) key_path = recent.keyPath
    }
  })

  /** Fill the form from a saved connection (tap-to-reconnect), and load its
   * stored password (if any) so the reconnect only needs the OTP. */
  function pick_saved(c: SavedConnection): void {
    label = c.label ?? ``
    host = c.host
    port = c.port
    username = c.username
    method = c.method
    key_path = c.keyPath ?? ``
    error_msg = ``
    auto_password = ``
    transport
      .keyLoad(`pw:${c.host}:${c.port}:${c.username}`)
      .then((pw) => {
        if (pw) auto_password = pw
      })
      .catch(() => {
        /* no stored password / desktop transport — type it manually */
      })
  }

  /** Clear the form to enter a brand-new cluster (the form otherwise prefills
   * from the most-recent saved connection). */
  function new_connection(): void {
    label = ``
    host = ``
    port = 22
    username = ``
    method = `keyboard-interactive`
    key_path = ``
    password = ``
    passphrase = ``
    auto_password = ``
    error_msg = ``
  }

  /** Delete a saved connection (does not touch any stored key material). */
  function delete_saved(id: string, e: Event): void {
    e.stopPropagation()
    saved = removeConnection(id)
  }

  function persist_non_secrets(): void {
    saved = upsertConnection(
      { host, port, username, method, keyPath: key_path, label },
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
      otp_pending_id = r.pendingId
      otp_prompts = r.prompts
      otp_instructions = r.instructions
      // OTP-only reconnect: if this round is just the account-password prompt and
      // we have a saved password, answer it silently and only surface later
      // rounds (the Duo passcode) to the user.
      if (auto_password && r.prompts.length === 1 && is_password_prompt(r.prompts[0])) {
        used_saved_pw = true
        const pw = auto_password
        auto_password = ``
        void submit_otp([pw])
        return
      }
      otp_visible = true
      otp_busy = false
      return
    }
    if (r.connected && r.sessionId) {
      otp_visible = false
      otp_busy = false
      persist_non_secrets()
      // Offer to save the password (once) so the next reconnect is OTP-only.
      // Park until the user decides — calling on_connected swaps us out.
      if (captured_password && !used_saved_pw) {
        pending_session = r.sessionId
        pending_pw = captured_password
        captured_password = ``
        save_prompt_visible = true
        return
      }
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
    used_saved_pw = false
    captured_password = method === `password` ? password : ``
    // Robustly load a saved password for THIS endpoint (so OTP-only reconnect
    // works even when the form was filled manually, not via a saved-list tap).
    if (!auto_password) {
      try {
        const pw = await transport.keyLoad(endpoint_pw_key())
        if (pw) {
          auto_password = pw
          // password method: fill the form value directly; keyboard-interactive
          // uses auto_password to answer the password prompt round.
          if (method === `password` && !password) password = pw
        }
      } catch {
        /* no stored password / desktop transport */
      }
    }
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
    // Remember the response to a password prompt so we can offer to save it.
    otp_prompts.forEach((p, i) => {
      if (is_password_prompt(p) && responses[i]) captured_password = responses[i]
    })
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

  function finish_connect(): void {
    save_prompt_visible = false
    const id = pending_session
    pending_session = ``
    pending_pw = ``
    on_connected?.(id)
  }

  /** Save the password (encrypted) for OTP-only reconnect, then continue. */
  async function save_password_yes(): Promise<void> {
    try {
      await transport.keyStore(endpoint_pw_key(), pending_pw)
    } catch {
      /* store unavailable — proceed without saving */
    }
    finish_connect()
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
        <div class="saved-head">
          <span class="saved-label">Saved</span>
          <button type="button" class="saved-new" onclick={new_connection}>+ New</button>
        </div>
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
      <label class="field name-field">
        <span>Name (optional)</span>
        <input
          type="text"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          placeholder="Expanse"
          bind:value={label}
        />
      </label>

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

{#if save_prompt_visible}
  <div class="sp-overlay" role="dialog" aria-modal="true">
    <div class="sp-card">
      <div class="sp-title">Save password for this cluster?</div>
      <div class="sp-body">
        Next time you connect to <b>{username}@{host}</b> you'll only need the
        one-time passcode (OTP). The password is encrypted on this device.
      </div>
      <div class="sp-actions">
        <button type="button" class="sp-no" onclick={finish_connect}>Not now</button>
        <button type="button" class="sp-yes" onclick={save_password_yes}>Save password</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .sp-overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.6);
  }
  .sp-card {
    width: 100%;
    max-width: 420px;
    padding: 20px;
    background: var(--surface-bg, #1a1a2e);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 12px;
  }
  .sp-title {
    font-size: 1.05em;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
    margin-bottom: 10px;
  }
  .sp-body {
    font-size: 0.9em;
    line-height: 1.5;
    color: var(--text-color-muted, #cbd5e1);
    margin-bottom: 18px;
  }
  .sp-actions {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }
  .sp-no,
  .sp-yes {
    min-height: 44px;
    padding: 0 16px;
    font-size: 15px;
    font-weight: 600;
    border-radius: 8px;
    cursor: pointer;
  }
  .sp-no {
    color: var(--text-color-muted, #94a3b8);
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.18);
  }
  .sp-yes {
    color: #fff;
    background: var(--accent-color, #0a84ff);
    border: 1px solid var(--accent-color, #0a84ff);
  }
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
  .saved-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .saved-label {
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
  .saved-new {
    min-height: 32px;
    padding: 0 12px;
    font-size: 13px;
    font-weight: 600;
    color: var(--accent-color, #3b82f6);
    background: rgba(59, 130, 246, 0.1);
    border: 1px solid rgba(59, 130, 246, 0.3);
    border-radius: 8px;
    cursor: pointer;
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

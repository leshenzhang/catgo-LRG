<!--
  KeySetup.svelte — onboarding flow for SSH-key passwordless login.

  Shown AFTER the first successful interactive (Duo) connect. Drives the three
  steps:
    1. generate  -> transport.keygen()                       (on-device ed25519)
    2. install   -> transport.installPubkey(session_id, pub) (~/.ssh/authorized_keys)
    3. store     -> transport.keyStore(endpointKey, priv)    (wrapped at rest)

  The private key NEVER leaves the device unencrypted at rest and is dropped from
  memory as soon as it is stored. On success the cluster accepts publickey auth
  for future connects (Duo may still be required by the server — removing the
  password is the win). Emits `on_done(installed)` when finished or skipped.

  STANDALONE: owns its own flow state; the parent (MobileShell) decides when to
  mount it. Props are the live session + endpoint descriptor.
-->
<script lang="ts">
  import { transport } from '$lib/api/transport'
  import { t } from '$lib/i18n/index.svelte'

  interface Props {
    /** Live SSH session id (from the just-completed connect). */
    session_id: string
    /** Remote host (for the endpoint key + display). */
    host: string
    /** Remote port. */
    port: number
    /** Authenticated username. */
    username: string
    /** Emitted when the flow finishes: `true` if a key was installed+stored,
     * `false` if the user skipped. */
    on_done?: (installed: boolean) => void
  }

  let { session_id, host, port, username, on_done }: Props = $props()

  type Phase = `idle` | `generating` | `installing` | `storing` | `done` | `error`

  let phase = $state<Phase>(`idle`)
  let error_msg = $state(``)

  /** Stable per-cluster identity used as the storage key (matches the Rust
   * sanitizer's accepted charset; it sanitizes anyway). */
  const endpoint_key = $derived(`${host}:${port}:${username}`)

  const busy = $derived(
    phase === `generating` || phase === `installing` || phase === `storing`,
  )

  async function run_setup(): Promise<void> {
    if (busy) return
    error_msg = ``
    let priv_key = ``
    try {
      phase = `generating`
      const pair = await transport.keygen()
      priv_key = pair.privateOpenssh

      phase = `installing`
      await transport.installPubkey(session_id, pair.publicOpenssh)

      phase = `storing`
      await transport.keyStore(endpoint_key, priv_key)

      phase = `done`
      on_done?.(true)
    } catch (e: unknown) {
      phase = `error`
      error_msg = e instanceof Error ? e.message : String(e)
    } finally {
      // Drop the private key from memory regardless of outcome.
      priv_key = ``
    }
  }

  function skip(): void {
    if (busy) return
    on_done?.(false)
  }

  const status_text = $derived(
    phase === `generating`
      ? t(`mobile.ks_generating`)
      : phase === `installing`
        ? t(`mobile.ks_installing`, { host })
        : phase === `storing`
          ? t(`mobile.ks_storing`)
          : phase === `done`
            ? t(`mobile.ks_done`)
            : ``,
  )
</script>

<div class="ks-overlay" role="dialog" aria-modal="true" aria-label={t(`mobile.ks_aria`)}>
  <div class="ks-card">
    <div class="ks-title">{t(`mobile.ks_title`)}</div>

    <p class="ks-body">
      {t(`mobile.ks_body`, { user: `${username}@${host}` })}
    </p>

    {#if status_text}
      <div class="ks-status" class:done={phase === `done`} aria-live="polite">
        {#if busy}<span class="ks-spinner" aria-hidden="true"></span>{/if}
        <span>{status_text}</span>
      </div>
    {/if}

    {#if phase === `error` && error_msg}
      <div class="ks-error" role="alert">{error_msg}</div>
    {/if}

    <div class="ks-actions">
      <button type="button" class="ks-btn skip" disabled={busy} onclick={skip}>
        {phase === `done` ? t(`mobile.ks_close`) : t(`mobile.ks_not_now`)}
      </button>
      {#if phase !== `done`}
        <button type="button" class="ks-btn go" disabled={busy} onclick={run_setup}>
          {phase === `error` ? t(`mobile.ks_try_again`) : busy ? t(`mobile.ks_working`) : t(`mobile.ks_set_up`)}
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .ks-overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
    background: rgba(0, 0, 0, 0.6);
  }
  .ks-card {
    width: 100%;
    max-width: 440px;
    background: var(--surface-bg, #1a1a2e);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 12px;
    padding: 20px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5);
  }
  .ks-title {
    font-size: 1.1em;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
    margin-bottom: 10px;
  }
  .ks-body {
    font-size: 0.9em;
    line-height: 1.5;
    color: var(--text-color-muted, #94a3b8);
    margin: 0 0 14px;
  }
  .ks-status {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.85em;
    color: var(--text-color, #e0e0e0);
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    padding: 10px 12px;
    margin-bottom: 12px;
  }
  .ks-status.done {
    color: #4ade80;
    border-color: rgba(74, 222, 128, 0.3);
    background: rgba(74, 222, 128, 0.08);
  }
  .ks-spinner {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    border: 2px solid rgba(255, 255, 255, 0.25);
    border-top-color: var(--accent-color, #3b82f6);
    border-radius: 50%;
    animation: ks-spin 0.8s linear infinite;
  }
  @keyframes ks-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .ks-error {
    font-size: 0.85em;
    color: #ff6b6b;
    background: rgba(255, 107, 107, 0.1);
    border: 1px solid rgba(255, 107, 107, 0.3);
    border-radius: 8px;
    padding: 8px 10px;
    margin-bottom: 12px;
    white-space: pre-wrap;
  }
  .ks-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .ks-btn {
    min-height: 48px;
    padding: 0 18px;
    font-size: 15px;
    font-weight: 600;
    border-radius: 8px;
    cursor: pointer;
    border: 1px solid rgba(255, 255, 255, 0.14);
  }
  .ks-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .ks-btn.skip {
    color: var(--text-color, #e0e0e0);
    background: rgba(255, 255, 255, 0.06);
  }
  .ks-btn.go {
    color: #fff;
    background: var(--accent-color, #0a84ff);
    border-color: var(--accent-color, #0a84ff);
  }
</style>

<!--
  MobileShell.svelte — top-level mobile UI for the tauri-ssh transport.

  Renders {@link MobileConnect} until a session is established, then swaps to
  {@link MobileTerminal} with a slim header that shows the session and offers
  Disconnect (which tears the terminal down and returns to the connect form).

  This is the SINGLE component mounted behind the `isMobile()` gate in the app
  entry; the desktop UI is never affected.
-->
<script lang="ts">
  import { transport } from '$lib/api/transport'
  import MobileConnect from './MobileConnect.svelte'
  import MobileTerminal from './MobileTerminal.svelte'
  import MobileFiles from './MobileFiles.svelte'
  import MobileJobs from './MobileJobs.svelte'
  import KeySetup from './KeySetup.svelte'
  import { loadConnections } from './connections'

  let session_id = $state<string | null>(null)
  let shell_el: HTMLDivElement | undefined = $state()

  // Which screen the connected view shows. Terminal / Files / Jobs all share the
  // one live session. Files/Jobs are lazy-mounted on first open (so they don't
  // hit the login node until asked) and then kept alive (hidden) so switching
  // back doesn't refetch and the terminal keeps its PTY + scrollback.
  let active_tab = $state<`terminal` | `files` | `jobs`>(`terminal`)
  let files_seen = $state(false)
  let jobs_seen = $state(false)

  function set_tab(tab: `terminal` | `files` | `jobs`): void {
    active_tab = tab
    if (tab === `files`) files_seen = true
    if (tab === `jobs`) jobs_seen = true
  }

  // Passwordless-login onboarding (shown once after the first connect to an
  // endpoint that has no stored key yet).
  let ks_visible = $state(false)
  let ks_host = $state(``)
  let ks_port = $state(22)
  let ks_user = $state(``)

  // Keep the shell sized to the VISUAL viewport so the soft keyboard never
  // overlaps the terminal. `100dvh` (the CSS fallback below) excludes browser
  // chrome but NOT the IME inset on Android WebView, so when the keyboard opens
  // the terminal would otherwise stay full-height and get covered. Binding the
  // root height to `visualViewport.height` shrinks the whole column the moment
  // the keyboard appears; MobileTerminal's ResizeObserver then re-fits xterm
  // above it. `offsetTop` pins the column when the WebView pans rather than
  // resizing. Desktop is unaffected — this component only mounts on mobile.
  $effect(() => {
    const vv = typeof window !== `undefined` ? window.visualViewport : null
    const el = shell_el
    if (!vv || !el) return

    const apply = (): void => {
      el.style.height = `${vv.height}px`
      el.style.transform = vv.offsetTop ? `translateY(${vv.offsetTop}px)` : ``
    }
    apply()
    vv.addEventListener(`resize`, apply)
    vv.addEventListener(`scroll`, apply)
    return () => {
      vv.removeEventListener(`resize`, apply)
      vv.removeEventListener(`scroll`, apply)
      el.style.height = ``
      el.style.transform = ``
    }
  })

  function on_connected(id: string): void {
    session_id = id
    active_tab = `terminal`

    // Offer passwordless-login setup once per endpoint. MobileConnect persists
    // the connection on success, so the most-recent entry gives us host/port/
    // user without MobileConnect having to emit them. Only prompt when no key is
    // stored for this endpoint yet; on the desktop/http transport `keyLoad`
    // rejects, so the prompt never shows there.
    const recent = loadConnections()[0]
    if (!recent) return
    ks_host = recent.host
    ks_port = recent.port
    ks_user = recent.username
    const endpoint = `${recent.host}:${recent.port}:${recent.username}`
    transport
      .keyLoad(endpoint)
      .then((stored) => {
        ks_visible = stored == null && recent.method !== `publickey`
      })
      .catch(() => {
        ks_visible = false
      })
  }

  function disconnect(): void {
    // Dropping session_id unmounts the active screen, whose onDestroy tears down
    // its PTY/listeners. The SSH session itself stays alive on the Rust side;
    // reconnecting reuses the same connect flow.
    session_id = null
    ks_visible = false
    active_tab = `terminal`
    files_seen = false
    jobs_seen = false
  }
</script>

<div class="mobile-shell" bind:this={shell_el}>
  {#if session_id}
    <header class="ms-header">
      <nav class="ms-tabs">
        <button
          type="button"
          class:active={active_tab === `terminal`}
          onclick={() => set_tab(`terminal`)}>Terminal</button
        >
        <button
          type="button"
          class:active={active_tab === `files`}
          onclick={() => set_tab(`files`)}>Files</button
        >
        <button
          type="button"
          class:active={active_tab === `jobs`}
          onclick={() => set_tab(`jobs`)}>Jobs</button
        >
      </nav>
      <button type="button" class="ms-disconnect" onclick={disconnect}>
        Disconnect
      </button>
    </header>
    <div class="ms-body">
      <div class="ms-pane" class:hidden={active_tab !== `terminal`}>
        <MobileTerminal {session_id} />
      </div>
      {#if files_seen}
        <div class="ms-pane" class:hidden={active_tab !== `files`}>
          <MobileFiles {session_id} />
        </div>
      {/if}
      {#if jobs_seen}
        <div class="ms-pane" class:hidden={active_tab !== `jobs`}>
          <MobileJobs {session_id} />
        </div>
      {/if}
    </div>
  {:else}
    <MobileConnect {on_connected} />
  {/if}
</div>

{#if ks_visible && session_id}
  <KeySetup
    {session_id}
    host={ks_host}
    port={ks_port}
    username={ks_user}
    on_done={() => (ks_visible = false)}
  />
{/if}

<style>
  .mobile-shell {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100vh;
    height: 100dvh;
    min-height: 0;
    background: var(--page-bg, #0e1117);
  }
  .ms-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-shrink: 0;
    padding: 6px 10px;
    padding-top: max(6px, env(safe-area-inset-top));
    background: rgba(0, 0, 0, 0.3);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .ms-tabs {
    display: flex;
    flex: 1;
    min-width: 0;
    gap: 2px;
  }
  .ms-tabs button {
    flex: 1;
    min-height: 40px;
    font-size: 14px;
    font-weight: 600;
    color: var(--text-color-muted, #94a3b8);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    cursor: pointer;
  }
  .ms-tabs button.active {
    color: var(--text-color, #e0e0e0);
    border-bottom-color: var(--accent-color, #3b82f6);
  }
  .ms-disconnect {
    min-height: 36px;
    padding: 0 14px;
    font-size: 14px;
    color: #ff6b6b;
    background: rgba(255, 107, 107, 0.1);
    border: 1px solid rgba(255, 107, 107, 0.3);
    border-radius: 8px;
    cursor: pointer;
  }
  .ms-body {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .ms-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .ms-pane.hidden {
    display: none;
  }
  .ms-body :global(.mobile-terminal),
  .ms-body :global(.mobile-files),
  .ms-body :global(.mobile-jobs) {
    flex: 1;
    min-width: 0;
  }
</style>

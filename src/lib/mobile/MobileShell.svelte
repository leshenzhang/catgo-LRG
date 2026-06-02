<!--
  MobileShell.svelte — top-level mobile UI for the tauri-ssh transport.

  Renders {@link MobileConnect} until a session is established, then swaps to
  {@link MobileTerminal} with a slim header that shows the session and offers
  Disconnect (which tears the terminal down and returns to the connect form).

  This is the SINGLE component mounted behind the `isMobile()` gate in the app
  entry; the desktop UI is never affected.
-->
<script lang="ts">
  import MobileConnect from './MobileConnect.svelte'
  import MobileTerminal from './MobileTerminal.svelte'

  let session_id = $state<string | null>(null)

  function on_connected(id: string): void {
    session_id = id
  }

  function disconnect(): void {
    // Dropping session_id unmounts MobileTerminal, whose onDestroy calls
    // transport.ptyClose. The SSH session itself stays alive on the Rust side;
    // reconnecting reuses the same connect flow.
    session_id = null
  }
</script>

<div class="mobile-shell">
  {#if session_id}
    <header class="ms-header">
      <span class="ms-title">SSH session</span>
      <button type="button" class="ms-disconnect" onclick={disconnect}>
        Disconnect
      </button>
    </header>
    <div class="ms-body">
      <MobileTerminal {session_id} />
    </div>
  {:else}
    <MobileConnect {on_connected} />
  {/if}
</div>

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
    flex-shrink: 0;
    padding: 10px 14px;
    padding-top: max(10px, env(safe-area-inset-top));
    background: rgba(0, 0, 0, 0.3);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .ms-title {
    font-size: 0.9em;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
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
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .ms-body :global(.mobile-terminal) {
    flex: 1;
  }
</style>

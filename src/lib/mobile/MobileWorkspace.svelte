<!--
  MobileWorkspace.svelte — purpose-built mobile layout for CatGo.

  Terminal-first workspace that hosts BOTH the real structure editor and the
  russh SSH terminal, with switchable layouts. Connecting to a cluster is
  OPTIONAL: the user can just view/edit a local structure (no backend, exactly
  like the web build), or connect for a remote terminal + remote files.

  Entry: a chooser — "Structure" (local, no connect) or "Terminal" (connect).
  Layouts: full-structure / full-terminal / split (horizontal or vertical).
  Flow: tap a structure in Files -> it loads in the structure pane; edit it;
  one-tap Save writes it back to the cluster (sftpWrite) when it came from there,
  else downloads it; one-tap back to the terminal.

  Mounted behind the isMobile() gate INSIDE App.svelte, so the app-root globals
  the editor needs (i18n, theme, <Toast/>) are already set up.
-->
<script lang="ts">
  import Structure from '$lib/structure/Structure.svelte'
  import OptimadeSearchModal from '$lib/structure/OptimadeSearchModal.svelte'
  import { parse_any_structure } from '$lib/structure/parsers/dispatch'
  import { structure_to_poscar } from '$lib/structure/export/offline-serialize'
  import { writeRemoteFile } from '$lib/api/hpc'
  import { transport } from '$lib/api/transport'
  import MobileConnect from './MobileConnect.svelte'
  import MobileTerminal from './MobileTerminal.svelte'
  import MobileFiles from './MobileFiles.svelte'
  import KeySetup from './KeySetup.svelte'
  import { loadConnections } from './connections'

  type Mode = `choose` | `structure` | `terminal` | `split-h` | `split-v`

  let mode = $state<Mode>(`choose`)
  let session_id = $state<string | null>(null)
  // The live structure (edited in place) + the saveable view (no PBC images).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let structure = $state<any>(undefined)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let saveable_structure = $state<any>(undefined)
  // Where the open structure came from, so Save knows to write it back.
  let remote_origin = $state<{ path: string; filename: string } | null>(null)
  let local_filename = $state(`structure.vasp`)
  let files_open = $state(false)
  let db_visible = $state(false)
  let save_msg = $state(``)

  // Hide the editor toolbar items that don't apply on mobile: server/HPC +
  // terminal are owned by MobileWorkspace (russh); workflow, plugin_hub and chat
  // are Python-backend-only (chat streams via /api/agent/stream) so they can't
  // work without the backend.
  // gesture (MediaPipe hand-tracking via camera) is dropped on mobile: the
  // front camera + hand model + the 3D editor together exhaust the WebView and
  // crash the app on this hardware.
  const HIDDEN_TOOLBAR = [`server`, `terminal`, `workflow`, `plugin_hub`, `chat`, `gesture`]

  // SSH-key passwordless onboarding (shown once per endpoint after first connect).
  let ks_visible = $state(false)
  let ks_host = $state(``)
  let ks_port = $state(22)
  let ks_user = $state(``)

  let file_input: HTMLInputElement | undefined = $state()
  let root_el: HTMLDivElement | undefined = $state()

  // Track the visual viewport so the terminal sits flush against the keyboard the
  // moment it opens (the native window-insets padding alone settles a frame late,
  // so without this the bar shows a gap until the keyboard is toggled once).
  $effect(() => {
    const vv = typeof window !== `undefined` ? window.visualViewport : null
    const el = root_el
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

  let term_cwd = $state(``)
  const has_structure = $derived(structure != null)

  // Auto-dismiss the save/notice banner so it never sticks permanently; a ✕ also
  // clears it immediately.
  $effect(() => {
    if (!save_msg) return
    const t = setTimeout(() => {
      save_msg = ``
    }, 6000)
    return () => clearTimeout(t)
  })
  const can_save = $derived(has_structure && (saveable_structure != null || structure != null))

  function set_structure(content: string, filename: string, origin: { path: string } | null): void {
    const parsed = parse_any_structure(content, filename)
    if (!parsed) {
      save_msg = `Could not parse ${filename} as a structure.`
      return
    }
    structure = parsed
    saveable_structure = undefined
    local_filename = filename
    remote_origin = origin ? { path: origin.path, filename } : null
    save_msg = ``
    files_open = false
    if (mode === `choose` || mode === `terminal`) mode = `structure`
  }

  // ── Local file open (no cluster needed) ──
  function open_local(): void {
    file_input?.click()
  }
  async function on_local_file(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement
    const f = input.files?.[0]
    if (!f) return
    const text = await f.text()
    set_structure(text, f.name, null)
    input.value = ``
  }

  // ── Remote structure open (from the Files browser) ──
  function open_remote_structure(content: string, filename: string, path: string): void {
    set_structure(content, filename, { path })
  }

  // ── Database import (OPTIMADE / Materials Project / PubChem) ──
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  function on_db_import(s: any): void {
    structure = s
    saveable_structure = undefined
    local_filename = `POSCAR`
    remote_origin = null
    save_msg = ``
    db_visible = false
    if (mode === `choose` || mode === `terminal`) mode = `structure`
  }

  // ── Save: write back to the cluster, or download locally ──
  async function save(): Promise<void> {
    const s = saveable_structure ?? structure
    if (!s) return
    let text: string
    try {
      text = structure_to_poscar(s, `CatGo mobile`)
    } catch (err) {
      save_msg = err instanceof Error ? err.message : String(err)
      return
    }
    if (remote_origin && session_id) {
      const r = await writeRemoteFile(session_id, remote_origin.path, text)
      save_msg = r.success ? `Saved to ${remote_origin.path}` : `Save failed: ${r.message}`
    } else {
      const blob = new Blob([text], { type: `text/plain` })
      const url = URL.createObjectURL(blob)
      const a = document.createElement(`a`)
      a.href = url
      a.download = local_filename || `POSCAR`
      a.click()
      URL.revokeObjectURL(url)
      save_msg = `Downloaded ${a.download}`
    }
  }

  // Drop the session → the terminal pane shows the connect form again (saved
  // connections + OTP-only reconnect still apply). The structure stays loaded.
  function disconnect(): void {
    session_id = null
    ks_visible = false
    files_open = false
    if (!has_structure) mode = `terminal`
  }

  function on_connected(id: string): void {
    session_id = id
    if (mode === `choose`) mode = `terminal`

    // Offer SSH-key passwordless setup once per endpoint (skip if already keyed
    // or the connection already used a public key). MobileConnect persists the
    // connection on success, so the most-recent entry gives host/port/user.
    const recent = loadConnections()[0]
    if (!recent || recent.method === `publickey`) return
    ks_host = recent.host
    ks_port = recent.port
    ks_user = recent.username
    transport
      .keyLoad(`${recent.host}:${recent.port}:${recent.username}`)
      .then((k) => {
        ks_visible = k == null
      })
      .catch(() => {
        ks_visible = false
      })
  }

  // Which panes are visible in the current layout.
  const show_structure = $derived(mode === `structure` || mode === `split-h` || mode === `split-v`)
  const show_terminal = $derived(mode === `terminal` || mode === `split-h` || mode === `split-v`)
</script>

<input
  bind:this={file_input}
  type="file"
  accept=".cif,.poscar,.vasp,.xyz,.extxyz,.json,.cube,.lammps,.data,*"
  onchange={on_local_file}
  hidden
/>

<div class="mw-root">
  {#if mode === `choose`}
    <div class="mw-choose">
      <div class="mw-choose-title">CatGo</div>
      <div class="mw-choose-sub">What do you want to do?</div>
      <button type="button" class="mw-choice" onclick={open_local}>
        <span class="mw-choice-icon">⬚</span>
        <span class="mw-choice-main">View / edit a structure</span>
        <span class="mw-choice-desc">Open a local file — no cluster needed</span>
      </button>
      <button type="button" class="mw-choice" onclick={() => (db_visible = true)}>
        <span class="mw-choice-icon">🗄</span>
        <span class="mw-choice-main">Import from a database</span>
        <span class="mw-choice-desc">Search OPTIMADE / Materials Project / PubChem</span>
      </button>
      <button type="button" class="mw-choice" onclick={() => (mode = `terminal`)}>
        <span class="mw-choice-icon">⌨</span>
        <span class="mw-choice-main">Connect to cluster</span>
        <span class="mw-choice-desc">SSH terminal + remote files</span>
      </button>
    </div>
  {:else}
    <!-- Top bar: layout switch + actions -->
    <header class="mw-bar">
      <div class="mw-tabs">
        <button type="button" class:active={mode === `structure`} onclick={() => (mode = `structure`)} title="Structure">⬚</button>
        <button type="button" class:active={mode === `split-v`} onclick={() => (mode = `split-v`)} title="Split (stacked)">⊟</button>
        <button type="button" class:active={mode === `split-h`} onclick={() => (mode = `split-h`)} title="Split (side by side)">⊞</button>
        <button type="button" class:active={mode === `terminal`} onclick={() => (mode = `terminal`)} title="Terminal">▭</button>
      </div>
      <div class="mw-actions">
        {#if session_id}
          <button type="button" class="mw-act" onclick={() => (files_open = true)} title="Remote files">📁</button>
        {/if}
        <button type="button" class="mw-act" onclick={open_local} title="Open local file">⬆</button>
        <button type="button" class="mw-act" onclick={() => (db_visible = true)} title="Import from database">🗄</button>
        {#if can_save}
          <button type="button" class="mw-act save" onclick={save} title="Save structure">💾</button>
        {/if}
        {#if session_id}
          <button type="button" class="mw-act disconnect" onclick={disconnect} title="Disconnect">⏏</button>
        {/if}
      </div>
    </header>

    {#if save_msg}
      <div class="mw-msg">
        <span>{save_msg}</span>
        <button type="button" class="mw-msg-x" aria-label="Dismiss" onclick={() => (save_msg = ``)}>✕</button>
      </div>
    {/if}

    <div class="mw-body" class:split-h={mode === `split-h`} class:split-v={mode === `split-v`}>
      {#if show_structure}
        <div class="mw-pane mw-struct">
          {#if has_structure}
            <Structure
              bind:structure
              bind:saveable_structure
              show_controls={true}
              fullscreen_toggle={false}
              allow_file_drop={false}
              persist_settings={false}
              hidden_toolbar_items={HIDDEN_TOOLBAR}
            />
          {:else}
            <div class="mw-empty">
              <p>No structure loaded.</p>
              <button type="button" class="mw-open-btn" onclick={open_local}>Open local file</button>
              {#if session_id}
                <button type="button" class="mw-open-btn" onclick={() => (files_open = true)}>Open from cluster</button>
              {/if}
            </div>
          {/if}
        </div>
      {/if}

      {#if show_terminal}
        <div class="mw-pane mw-term">
          {#if session_id}
            <MobileTerminal {session_id} on_cwd={(p) => (term_cwd = p)} />
          {:else}
            <div class="mw-connect">
              <MobileConnect {on_connected} />
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  {#if files_open && session_id}
    <div class="mw-files-overlay">
      <header class="mw-files-head">
        <span>Remote files</span>
        <button type="button" onclick={() => (files_open = false)}>✕</button>
      </header>
      <div class="mw-files-body">
        <MobileFiles {session_id} follow_path={term_cwd} on_open_structure={open_remote_structure} />
      </div>
    </div>
  {/if}

  {#if ks_visible && session_id}
    <KeySetup
      {session_id}
      host={ks_host}
      port={ks_port}
      username={ks_user}
      on_done={() => (ks_visible = false)}
    />
  {/if}
</div>

{#if db_visible}
  <OptimadeSearchModal
    visible={db_visible}
    onclose={() => (db_visible = false)}
    onimport={on_db_import}
  />
{/if}

<style>
  .mw-root {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--page-bg, #0e1117);
  }

  /* Entry chooser */
  .mw-choose {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 14px;
    padding: 24px;
  }
  .mw-choose-title {
    font-size: 1.8em;
    font-weight: 700;
    color: var(--text-color, #e0e0e0);
  }
  .mw-choose-sub {
    font-size: 0.95em;
    color: var(--text-color-muted, #94a3b8);
    margin-bottom: 8px;
  }
  .mw-choice {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 18px;
    text-align: left;
    background: var(--surface-bg, #1a1a2e);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 12px;
    cursor: pointer;
  }
  .mw-choice:active {
    border-color: var(--accent-color, #3b82f6);
  }
  .mw-choice-icon {
    font-size: 22px;
  }
  .mw-choice-main {
    font-size: 1.05em;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
  }
  .mw-choice-desc {
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }

  /* Top bar */
  .mw-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-shrink: 0;
    padding: 6px 10px;
    background: rgba(0, 0, 0, 0.3);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .mw-tabs,
  .mw-actions {
    display: flex;
    gap: 4px;
  }
  .mw-tabs button,
  .mw-act {
    min-width: 40px;
    min-height: 40px;
    font-size: 17px;
    color: var(--text-color-muted, #94a3b8);
    background: transparent;
    border: 1px solid transparent;
    border-radius: 8px;
    cursor: pointer;
  }
  .mw-tabs button.active {
    color: var(--accent-color, #3b82f6);
    border-color: var(--accent-color, #3b82f6);
  }
  .mw-act.save {
    color: #4ade80;
  }
  .mw-act.disconnect {
    color: #ff6b6b;
  }
  .mw-msg {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
    padding: 6px 8px 6px 12px;
    font-size: 0.82em;
    color: var(--text-color-muted, #cbd5e1);
    background: rgba(59, 130, 246, 0.1);
  }
  .mw-msg span {
    flex: 1;
    min-width: 0;
  }
  .mw-msg-x {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    font-size: 13px;
    color: var(--text-color-muted, #94a3b8);
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
  }

  /* Body / panes */
  .mw-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .mw-body.split-h {
    flex-direction: row;
  }
  .mw-body.split-v {
    flex-direction: column;
  }
  .mw-pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    position: relative;
    display: flex;
  }
  /* The editor's root (.structure-main) defaults to height:500px via
     --struct-height; override it so it fills the pane (no black gap / clipping
     in any layout). */
  .mw-struct {
    --struct-height: 100%;
    --struct-width: 100%;
    overflow: hidden;
  }
  .mw-struct :global(.structure-main) {
    height: 100%;
    width: 100%;
  }
  .mw-body.split-h .mw-struct,
  .mw-body.split-v .mw-struct {
    border-right: 1px solid rgba(255, 255, 255, 0.08);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .mw-struct :global(> *),
  .mw-term :global(.mobile-terminal),
  .mw-connect {
    flex: 1;
    min-width: 0;
    width: 100%;
  }
  .mw-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-color-muted, #94a3b8);
  }
  .mw-open-btn {
    min-height: 44px;
    padding: 0 18px;
    font-size: 15px;
    color: #fff;
    background: var(--accent-color, #0a84ff);
    border: none;
    border-radius: 8px;
    cursor: pointer;
  }
  .mw-connect {
    display: flex;
  }

  /* Remote files overlay */
  .mw-files-overlay {
    position: absolute;
    inset: 0;
    z-index: 100;
    display: flex;
    flex-direction: column;
    background: var(--page-bg, #0e1117);
  }
  .mw-files-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
    padding: 10px 14px;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
    background: rgba(0, 0, 0, 0.3);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .mw-files-head button {
    width: 36px;
    height: 36px;
    font-size: 16px;
    color: var(--text-color-muted, #94a3b8);
    background: transparent;
    border: none;
    cursor: pointer;
  }
  .mw-files-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .mw-files-body :global(.mobile-files) {
    flex: 1;
  }
</style>

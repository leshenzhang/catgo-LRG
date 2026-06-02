<!--
  MobileFiles.svelte — read-only SFTP file browser for the mobile (tauri-ssh)
  transport.

  Given a live `session_id`, it resolves the remote home directory once
  (`echo $HOME`, falling back to `.`), lists it via `transport.sftpList`, and
  renders directories-first-then-files rows. Tapping a directory navigates into
  it; an `..` row (and the breadcrumb) navigate to the parent. Tapping a file
  opens {@link MobileFileViewer} (read-only text viewer with a binary guard).

  Loading / error states mirror MobileTerminal's mt-status / mt-error styling.
  A manual Refresh button re-lists the current directory.

  NEW + standalone: read-only browse + view; never touches the desktop path.
-->
<script lang="ts">
  import { transport, type SftpEntry } from '$lib/api/transport'
  import MobileFileViewer from './MobileFileViewer.svelte'
  import { humanSize, joinPath, parentPath } from './files-util'

  interface Props {
    /** Live HPC session id (from MobileConnect). */
    session_id: string
  }

  let { session_id }: Props = $props()

  let cwd = $state(``)
  let entries = $state<SftpEntry[]>([])
  let status = $state<`init` | `loading` | `ready` | `error`>(`init`)
  let error_msg = $state(``)

  // Open file (null => showing the listing).
  let open_file = $state<SftpEntry | null>(null)

  // Directories first, then files; each group alphabetical (case-insensitive).
  const sorted = $derived(
    [...entries].sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
      return a.name.localeCompare(b.name, undefined, { sensitivity: `base` })
    }),
  )

  const at_root = $derived(cwd === `/`)

  // Breadcrumb segments with their cumulative absolute paths.
  const crumbs = $derived.by(() => {
    if (!cwd || cwd === `/`) return [{ label: `/`, path: `/` }]
    const parts = cwd.split(`/`).filter(Boolean)
    let acc = ``
    const out = [{ label: `/`, path: `/` }]
    for (const p of parts) {
      acc = `${acc}/${p}`
      out.push({ label: p, path: acc })
    }
    return out
  })

  async function list_dir(path: string): Promise<void> {
    status = `loading`
    error_msg = ``
    try {
      const items = await transport.sftpList(session_id, path)
      // Hide the self/parent dot-entries; we provide our own `..` row.
      entries = items.filter((e) => e.name !== `.` && e.name !== `..`)
      cwd = path
      status = `ready`
    } catch (e: unknown) {
      error_msg = e instanceof Error ? e.message : String(e)
      status = `error`
    }
  }

  /** Resolve $HOME once, then list it. Falls back to `.` then `/`. */
  async function init(): Promise<void> {
    status = `loading`
    error_msg = ``
    let start = `.`
    try {
      const r = await transport.exec(session_id, `echo $HOME`, 10_000)
      const home = r.stdout.trim()
      if (r.code === 0 && home.startsWith(`/`)) start = home
    } catch {
      /* fall back to `.` */
    }
    await list_dir(start)
    // If `.` somehow failed, last-resort to root so the browser is never stuck.
    if (status === `error` && start !== `/`) await list_dir(`/`)
  }

  $effect(() => {
    // Re-init whenever the session changes.
    void session_id
    init()
  })

  function enter(entry: SftpEntry): void {
    if (entry.isDir) {
      open_file = null
      list_dir(entry.path || joinPath(cwd, entry.name))
    } else {
      open_file = entry
    }
  }

  function go_up(): void {
    open_file = null
    list_dir(parentPath(cwd))
  }

  function go_crumb(path: string): void {
    open_file = null
    if (path !== cwd) list_dir(path)
  }

  function refresh(): void {
    if (cwd) list_dir(cwd)
  }
</script>

<div class="mobile-files">
  <header class="mf-bar">
    <nav class="mf-crumbs" aria-label="Path">
      {#each crumbs as c, i (c.path)}
        {#if i > 0}<span class="mf-sep">/</span>{/if}
        <button
          type="button"
          class="mf-crumb"
          class:current={c.path === cwd}
          onclick={() => go_crumb(c.path)}
        >
          {c.label}
        </button>
      {/each}
    </nav>
    <button
      type="button"
      class="mf-refresh"
      aria-label="Refresh"
      onclick={refresh}
      disabled={status === `loading`}
    >
      ⟳
    </button>
  </header>

  <div class="mf-body">
    {#if status === `loading` && entries.length === 0}
      <div class="mf-status">Loading…</div>
    {:else if status === `error`}
      <div class="mf-error">{error_msg}</div>
    {:else}
      <ul class="mf-list">
        {#if !at_root}
          <li>
            <button type="button" class="mf-row mf-up" onclick={go_up}>
              <span class="mf-icon">↰</span>
              <span class="mf-name">..</span>
              <span class="mf-meta">parent</span>
            </button>
          </li>
        {/if}
        {#each sorted as entry (entry.path || entry.name)}
          <li>
            <button type="button" class="mf-row" onclick={() => enter(entry)}>
              <span class="mf-icon">{entry.isDir ? `📁` : `📄`}</span>
              <span class="mf-name" class:dir={entry.isDir}>{entry.name}</span>
              <span class="mf-meta">
                {entry.isDir ? `` : humanSize(entry.size)}
              </span>
            </button>
          </li>
        {/each}
        {#if sorted.length === 0}
          <li class="mf-empty">Empty directory</li>
        {/if}
      </ul>
    {/if}
  </div>

  {#if open_file}
    <MobileFileViewer
      {session_id}
      path={open_file.path || joinPath(cwd, open_file.name)}
      size={open_file.size}
      on_close={() => (open_file = null)}
    />
  {/if}
</div>

<style>
  .mobile-files {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--page-bg, #0e1117);
  }
  .mf-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    padding: 8px 10px;
    background: rgba(0, 0, 0, 0.3);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .mf-crumbs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    white-space: nowrap;
  }
  .mf-sep {
    flex-shrink: 0;
    color: var(--text-color-muted, #94a3b8);
    padding: 0 2px;
  }
  .mf-crumb {
    flex-shrink: 0;
    min-height: 36px;
    padding: 0 6px;
    font-size: 14px;
    color: var(--accent-color, #3b82f6);
    background: transparent;
    border: none;
    cursor: pointer;
  }
  .mf-crumb.current {
    color: var(--text-color, #e0e0e0);
    font-weight: 600;
  }
  .mf-refresh {
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    font-size: 20px;
    line-height: 1;
    color: var(--text-color, #e0e0e0);
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 8px;
    cursor: pointer;
  }
  .mf-refresh:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .mf-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
  }
  .mf-status {
    padding: 12px;
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
  .mf-error {
    padding: 12px;
    font-size: 0.85em;
    color: #ff6b6b;
  }
  .mf-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .mf-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 48px;
    padding: 8px 14px;
    text-align: left;
    color: var(--text-color, #e0e0e0);
    background: transparent;
    border: none;
    border-bottom: 1px solid rgba(255, 255, 255, 0.05);
    cursor: pointer;
  }
  .mf-row:active {
    background: rgba(59, 130, 246, 0.12);
  }
  .mf-icon {
    flex-shrink: 0;
    width: 22px;
    font-size: 16px;
    text-align: center;
  }
  .mf-name {
    flex: 1;
    min-width: 0;
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mf-name.dir {
    color: var(--accent-color, #3b82f6);
  }
  .mf-meta {
    flex-shrink: 0;
    font-size: 0.78em;
    color: var(--text-color-muted, #94a3b8);
    font-variant-numeric: tabular-nums;
  }
  .mf-up .mf-name {
    color: var(--text-color-muted, #94a3b8);
  }
  .mf-empty {
    padding: 16px 14px;
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
</style>

<!--
  MobileFileViewer.svelte — read-only text viewer for a single remote file,
  opened from {@link MobileFiles}.

  Given a live `session_id` and a remote `path`, it reads the file via
  `transport.sftpRead(session, path, MAX_BYTES)` and shows the (UTF-8 lossy)
  contents in a monospace <pre> scroll area. Obviously-binary files (detected
  by extension or NUL bytes in the decoded text) are NOT rendered — instead the
  viewer shows the size and a short note, so we never dump megabytes of mojibake
  into the DOM.

  NEW + standalone: read-only, never writes, never touches the desktop path.
-->
<script lang="ts">
  import { transport } from '$lib/api/transport'
  import { humanSize, isBinaryName } from './files-util'

  interface Props {
    /** Live HPC session id. */
    session_id: string
    /** Full remote path of the file to view. */
    path: string
    /** Size in bytes (from the listing) for the binary-file summary. */
    size: number
    /** Close the viewer and return to the listing. */
    on_close: () => void
  }

  let { session_id, path, size, on_close }: Props = $props()

  // ~256 KB cap: enough for logs / INCAR / OUTCAR heads without OOM on mobile.
  const MAX_BYTES = 256 * 1024

  let status = $state<`loading` | `text` | `binary` | `error`>(`loading`)
  let content = $state(``)
  let truncated = $state(false)
  let error_msg = $state(``)

  const base_name = $derived(path.slice(path.lastIndexOf(`/`) + 1) || path)

  $effect(() => {
    let cancelled = false
    status = `loading`
    content = ``
    truncated = false
    error_msg = ``

    async function load(): Promise<void> {
      // Skip reads for files that are obviously binary by name.
      if (isBinaryName(base_name)) {
        if (!cancelled) status = `binary`
        return
      }
      try {
        const r = await transport.sftpRead(session_id, path, MAX_BYTES)
        if (cancelled) return
        // A NUL byte in the decoded text is a strong binary signal.
        if (/\x00/.test(r.content)) {
          status = `binary`
          return
        }
        content = r.content
        truncated = r.truncated
        status = `text`
      } catch (e: unknown) {
        if (!cancelled) {
          error_msg = e instanceof Error ? e.message : String(e)
          status = `error`
        }
      }
    }

    load()
    return () => {
      cancelled = true
    }
  })
</script>

<div class="fv-overlay" role="dialog" aria-modal="true" aria-label={base_name}>
  <header class="fv-header">
    <button type="button" class="fv-back" onclick={on_close} aria-label="Back to files">
      ‹ Back
    </button>
    <span class="fv-name" title={path}>{base_name}</span>
  </header>

  <div class="fv-body">
    {#if status === `loading`}
      <div class="fv-status">Loading…</div>
    {:else if status === `error`}
      <div class="fv-error">{error_msg}</div>
    {:else if status === `binary`}
      <div class="fv-binary">
        <div class="fv-binary-title">Binary file</div>
        <div class="fv-binary-note">
          {humanSize(size)} — not shown to avoid rendering non-text data.
        </div>
      </div>
    {:else}
      {#if truncated}
        <div class="fv-trunc">
          Showing the first {humanSize(MAX_BYTES)} — file is larger and was truncated.
        </div>
      {/if}
      <pre class="fv-pre">{content}</pre>
    {/if}
  </div>
</div>

<style>
  .fv-overlay {
    position: absolute;
    inset: 0;
    z-index: 5;
    display: flex;
    flex-direction: column;
    background: var(--page-bg, #0e1117);
  }
  .fv-header {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
    padding: 10px 12px;
    background: rgba(0, 0, 0, 0.3);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .fv-back {
    flex-shrink: 0;
    min-height: 36px;
    padding: 0 12px;
    font-size: 15px;
    color: var(--accent-color, #3b82f6);
    background: rgba(59, 130, 246, 0.1);
    border: 1px solid rgba(59, 130, 246, 0.3);
    border-radius: 8px;
    cursor: pointer;
  }
  .fv-name {
    flex: 1;
    min-width: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fv-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    -webkit-overflow-scrolling: touch;
  }
  .fv-status {
    padding: 12px;
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
  .fv-error {
    padding: 12px;
    font-size: 0.85em;
    color: #ff6b6b;
  }
  .fv-binary {
    padding: 24px 16px;
    text-align: center;
  }
  .fv-binary-title {
    font-size: 1em;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
    margin-bottom: 6px;
  }
  .fv-binary-note {
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
  .fv-trunc {
    padding: 8px 12px;
    font-size: 0.8em;
    color: #fbbf24;
    background: rgba(251, 191, 36, 0.08);
    border-bottom: 1px solid rgba(251, 191, 36, 0.2);
  }
  .fv-pre {
    margin: 0;
    padding: 12px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-color, #e0e0e0);
    white-space: pre;
    tab-size: 4;
  }
</style>

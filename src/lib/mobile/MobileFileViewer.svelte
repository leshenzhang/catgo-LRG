<!--
  MobileFileViewer.svelte — read-only previewer for a single remote file, opened
  from {@link MobileFiles} for NON-structure files (structures open in the 3D
  editor instead).

  Renders by kind, detected from the filename:
    - image (png/jpg/gif/webp/svg/bmp) -> <img> from base64 bytes
    - pdf                              -> <iframe> on a blob URL
    - markdown (md/markdown)           -> rendered HTML (markdown_to_html)
    - text (everything else readable)  -> monospace <pre>
  Truly-binary / oversized data is summarised instead of dumped.

  NEW + standalone: read-only, never writes, never touches the desktop path.
-->
<script lang="ts">
  import DOMPurify from 'dompurify'
  import { transport } from '$lib/api/transport'
  import { markdown_to_html } from '$lib/chat/markdown'
  import { humanSize, isBinaryName } from './files-util'

  interface Props {
    /** Live HPC session id. */
    session_id: string
    /** Full remote path of the file to view. */
    path: string
    /** Size in bytes (from the listing) for summaries / load guards. */
    size: number
    /** Close the viewer and return to the listing. */
    on_close: () => void
  }

  let { session_id, path, size, on_close }: Props = $props()

  // ~256 KB cap for text; binary (image/pdf) read whole but guarded by size.
  const MAX_BYTES = 256 * 1024
  const MAX_BINARY = 12 * 1024 * 1024

  type Kind = `loading` | `text` | `markdown` | `image` | `pdf` | `binary` | `error`
  let status = $state<Kind>(`loading`)
  let content = $state(``)
  let html = $state(``)
  let data_url = $state(``)
  let blob_url = $state(``)
  let truncated = $state(false)
  let error_msg = $state(``)

  const base_name = $derived(path.slice(path.lastIndexOf(`/`) + 1) || path)

  const IMAGE_EXTS = new Set([`png`, `jpg`, `jpeg`, `gif`, `webp`, `bmp`, `svg`, `ico`])
  function ext_of(name: string): string {
    const dot = name.lastIndexOf(`.`)
    return dot < 0 ? `` : name.slice(dot + 1).toLowerCase()
  }
  function mime_for(ext: string): string {
    return (
      ({ png: `image/png`, jpg: `image/jpeg`, jpeg: `image/jpeg`, gif: `image/gif`, webp: `image/webp`, bmp: `image/bmp`, svg: `image/svg+xml`, ico: `image/x-icon`, pdf: `application/pdf` } as Record<string, string>)[ext] ?? `application/octet-stream`
    )
  }
  function bytes_to_base64(bytes: Uint8Array): string {
    let bin = ``
    const chunk = 8192
    for (let i = 0; i < bytes.length; i += chunk) {
      bin += String.fromCharCode(...bytes.subarray(i, i + chunk))
    }
    return btoa(bin)
  }

  $effect(() => {
    let cancelled = false
    status = `loading`
    content = ``
    html = ``
    data_url = ``
    let local_blob = ``
    truncated = false
    error_msg = ``

    const ext = ext_of(base_name)
    const is_image = IMAGE_EXTS.has(ext)
    const is_pdf = ext === `pdf`
    const is_md = ext === `md` || ext === `markdown`

    async function load(): Promise<void> {
      try {
        if (is_image || is_pdf) {
          if (size > MAX_BINARY) {
            if (!cancelled) status = `binary`
            return
          }
          const bytes = await transport.sftpReadBytes(session_id, path)
          if (cancelled) return
          if (is_image) {
            data_url = `data:${mime_for(ext)};base64,${bytes_to_base64(bytes)}`
            status = `image`
          } else {
            const blob = new Blob([bytes], { type: `application/pdf` })
            local_blob = URL.createObjectURL(blob)
            blob_url = local_blob
            status = `pdf`
          }
          return
        }

        // Non-binary by name? then read as text (with a NUL-byte guard).
        if (isBinaryName(base_name)) {
          if (!cancelled) status = `binary`
          return
        }
        const r = await transport.sftpRead(session_id, path, MAX_BYTES)
        if (cancelled) return
        if (/\x00/.test(r.content)) {
          status = `binary`
          return
        }
        truncated = r.truncated
        if (is_md) {
          // Remote markdown is attacker-controllable (a shared cluster dir could
          // hold a malicious README). The chat markdown renderer does NOT scheme-
          // filter link/image URLs, so sanitize its output before {@html} to drop
          // javascript:/data: URLs, event handlers, and script/iframe injection.
          html = DOMPurify.sanitize(markdown_to_html(r.content), {
            ADD_ATTR: [`target`],
          })
          status = `markdown`
        } else {
          content = r.content
          status = `text`
        }
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
      if (local_blob) URL.revokeObjectURL(local_blob)
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

  <div class="fv-body" class:centered={status === `image` || status === `pdf`}>
    {#if status === `loading`}
      <div class="fv-status">Loading…</div>
    {:else if status === `error`}
      <div class="fv-error">{error_msg}</div>
    {:else if status === `binary`}
      <div class="fv-binary">
        <div class="fv-binary-title">Binary file</div>
        <div class="fv-binary-note">
          {humanSize(size)} — not shown (non-text or too large to preview).
        </div>
      </div>
    {:else if status === `image`}
      <img class="fv-img" src={data_url} alt={base_name} />
    {:else if status === `pdf`}
      <iframe class="fv-pdf" src={blob_url} title={base_name}></iframe>
    {:else if status === `markdown`}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      <div class="fv-md">{@html html}</div>
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
  .fv-body.centered {
    display: flex;
    overflow: hidden;
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
  .fv-img {
    margin: auto;
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .fv-pdf {
    flex: 1;
    width: 100%;
    height: 100%;
    border: none;
    background: #fff;
  }
  .fv-md {
    padding: 14px 16px;
    font-size: 15px;
    line-height: 1.6;
    color: var(--text-color, #e0e0e0);
    word-wrap: break-word;
    overflow-wrap: anywhere;
  }
  .fv-md :global(pre) {
    overflow-x: auto;
    padding: 10px;
    background: rgba(0, 0, 0, 0.3);
    border-radius: 6px;
  }
  .fv-md :global(img) {
    max-width: 100%;
  }
  .fv-md :global(a) {
    color: var(--accent-color, #3b82f6);
  }
  .fv-md :global(table) {
    display: block;
    overflow-x: auto;
    border-collapse: collapse;
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

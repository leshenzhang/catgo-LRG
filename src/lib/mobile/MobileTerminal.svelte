<!--
  MobileTerminal.svelte — interactive SSH terminal for the mobile (tauri-ssh)
  transport.

  Given a live `session_id`, it mounts xterm.js (same @xterm/xterm + addon-fit
  setup the desktop terminal uses), opens a remote PTY via
  `transport.ptyOpen(session, cols, rows, onData)`, and wires:

    - PTY output bytes  -> term.write(...)
    - term.onData       -> transport.ptyWrite (stdin)
    - ResizeObserver/fit -> transport.ptyResize
    - onDestroy         -> transport.ptyClose

  It also mounts the existing {@link MobileTerminalKeyBar} and forwards its
  `on_key(seq)` to `transport.ptyWrite` (encoded to bytes), and focuses the
  hidden xterm textarea so the soft keyboard appears.

  NEW + standalone: this never touches the desktop terminal / pty.ts.
-->
<script lang="ts">
  import { transport } from '$lib/api/transport'
  import MobileTerminalKeyBar from '$lib/structure/MobileTerminalKeyBar.svelte'

  interface Props {
    /** Live HPC session id (from MobileConnect). */
    session_id: string
  }

  let { session_id }: Props = $props()

  let container_el: HTMLDivElement | undefined = $state()
  let status = $state<`init` | `connected` | `error`>(`init`)
  let error_msg = $state(``)

  // Refs the keybar handler needs (set once the PTY is open).
  let channel_id: string | null = null
  let term_ref: { focus: () => void } | null = null
  const encoder = new TextEncoder()

  /** Forward a raw byte string (from the key bar) to the PTY as stdin. */
  function send_keys(seq: string): void {
    if (!channel_id) return
    transport.ptyWrite(session_id, channel_id, encoder.encode(seq)).catch(() => {})
    // Keep focus on the hidden textarea so the soft keyboard stays up.
    term_ref?.focus()
  }

  $effect(() => {
    if (!container_el) return

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let terminal: any = null
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    let fit_addon: any = null
    let observer: ResizeObserver | null = null
    let disposed = false
    let opened_channel: string | null = null

    async function init(): Promise<void> {
      try {
        const [xtermMod, fitMod] = await Promise.all([
          import(`@xterm/xterm`),
          import(`@xterm/addon-fit`),
        ])
        if (disposed) return
        await import(`@xterm/xterm/css/xterm.css`)

        const term_bg = `#0e1117`
        const term_fg = `#e0e0e0`
        const term = new xtermMod.Terminal({
          cursorBlink: true,
          fontSize: 13,
          fontFamily: `ui-monospace, SFMono-Regular, Menlo, monospace`,
          theme: {
            background: term_bg,
            foreground: term_fg,
            cursor: `#3b82f6`,
            cursorAccent: term_bg,
            selectionBackground: `rgba(59, 130, 246, 0.3)`,
          },
          allowProposedApi: true,
        })

        const fit = new fitMod.FitAddon()
        term.loadAddon(fit)
        term.open(container_el!)

        // Lay out before the first fit so column count is correct.
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => {
            if (!disposed) {
              try {
                fit.fit()
              } catch {
                /* ignore */
              }
            }
            resolve()
          }),
        )
        if (disposed) {
          term.dispose()
          return
        }

        terminal = term
        fit_addon = fit
        term_ref = term

        const cols = term.cols > 0 ? term.cols : 80
        const rows = term.rows > 0 ? term.rows : 24

        // Open the remote PTY; stream bytes straight into xterm.
        const ch = await transport.ptyOpen(session_id, cols, rows, (bytes) => {
          if (!disposed) term.write(bytes)
        })
        if (disposed) {
          transport.ptyClose(session_id, ch).catch(() => {})
          term.dispose()
          return
        }
        channel_id = ch
        opened_channel = ch
        status = `connected`

        // Stdin: xterm -> PTY.
        term.onData((data: string) => {
          if (!disposed && channel_id) {
            transport
              .ptyWrite(session_id, channel_id, encoder.encode(data))
              .catch(() => {})
          }
        })

        // Resize: keep the remote PTY in sync with xterm's grid.
        term.onResize(({ cols, rows }: { cols: number; rows: number }) => {
          if (!disposed && channel_id) {
            transport.ptyResize(session_id, channel_id, cols, rows).catch(() => {})
          }
        })

        // Container size changes -> re-fit (fit() triggers onResize above).
        observer = new ResizeObserver(() => {
          requestAnimationFrame(() => {
            if (!disposed && fit_addon) {
              try {
                fit_addon.fit()
              } catch {
                /* ignore */
              }
            }
          })
        })
        observer.observe(container_el!)

        // Focus the hidden textarea so the soft keyboard appears.
        term.focus()
      } catch (e: unknown) {
        if (!disposed) {
          status = `error`
          error_msg = e instanceof Error ? e.message : String(e)
        }
      }
    }

    init()

    return () => {
      disposed = true
      observer?.disconnect()
      const ch = opened_channel ?? channel_id
      if (ch) transport.ptyClose(session_id, ch).catch(() => {})
      channel_id = null
      term_ref = null
      terminal?.dispose()
    }
  })
</script>

<div class="mobile-terminal">
  <div class="mt-body" bind:this={container_el}>
    {#if status === `init`}
      <div class="mt-status">Opening shell…</div>
    {:else if status === `error`}
      <div class="mt-error">{error_msg}</div>
    {/if}
  </div>
  <MobileTerminalKeyBar on_key={send_keys} />
</div>

<style>
  .mobile-terminal {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--page-bg, #0e1117);
  }
  .mt-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }
  .mt-body :global(.xterm),
  .mt-body :global(.xterm-viewport),
  .mt-body :global(.xterm-screen) {
    height: 100%;
    width: 100%;
  }
  .mt-status {
    padding: 12px;
    font-size: 0.85em;
    color: var(--text-color-muted, #94a3b8);
  }
  .mt-error {
    padding: 12px;
    font-size: 0.85em;
    color: #ff6b6b;
  }
</style>

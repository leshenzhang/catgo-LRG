<!--
  MobileJobs.svelte — SLURM job panel for the mobile (tauri-ssh) transport.

  Given a live `session_id`, it lists the user's SLURM jobs via
  `transport.exec(session, "squeue ...")` with a robust pipe-delimited format,
  parses the lines into job objects, and renders one card per job with a colored
  state badge, elapsed/limit time, node count, and (when pending) the scheduler
  reason. Each card offers a Cancel action that runs `scancel <jobid>` behind a
  simple inline confirm, then refreshes.

  squeue is run with `--me`; if that flag is unsupported on the host, it falls
  back to `squeue -u $USER ...`. When squeue is missing entirely, a friendly
  "SLURM not available" message is shown instead.

  NEW + standalone: this never touches the desktop UI. It uses only the existing
  transport.exec method.
-->
<script lang="ts">
  import { transport } from '$lib/api/transport'

  interface Props {
    /** Live HPC session id (from MobileConnect). */
    session_id: string
  }

  let { session_id }: Props = $props()

  interface Job {
    id: string
    name: string
    state: string
    time: string
    timeLimit: string
    nodes: string
    reason: string
  }

  // Pipe-delimited squeue format: JobID|Name|State|Time|TimeLimit|Nodes|Reason.
  const SQUEUE_FMT = `%i|%j|%T|%M|%l|%D|%R`
  const SQUEUE_ME = `squeue --me --noheader -o '${SQUEUE_FMT}'`
  const SQUEUE_USER = `squeue -u "$USER" --noheader -o '${SQUEUE_FMT}'`

  let jobs = $state<Job[]>([])
  let status = $state<`init` | `loading` | `ready` | `error` | `unavailable`>(`init`)
  let error_msg = $state(``)
  // JobID currently awaiting cancel confirmation (inline), or null.
  let confirming = $state<string | null>(null)
  // JobID currently being cancelled (disables its buttons).
  let cancelling = $state<string | null>(null)

  /** Parse one pipe-delimited squeue line into a Job, or null if malformed. */
  function parse_line(line: string): Job | null {
    const parts = line.split(`|`)
    if (parts.length < 7) return null
    return {
      id: parts[0].trim(),
      name: parts[1].trim(),
      state: parts[2].trim(),
      time: parts[3].trim(),
      timeLimit: parts[4].trim(),
      nodes: parts[5].trim(),
      reason: parts[6].trim(),
    }
  }

  function parse_jobs(stdout: string): Job[] {
    return stdout
      .split(`\n`)
      .map((l) => l.trim())
      .filter((l) => l.length > 0)
      .map(parse_line)
      .filter((j): j is Job => j !== null)
  }

  /** True when stderr/stdout indicates squeue itself is not on the host. */
  function looks_missing(text: string): boolean {
    const t = text.toLowerCase()
    return (
      t.includes(`command not found`) ||
      t.includes(`not found`) ||
      t.includes(`no such file`)
    )
  }

  async function refresh(): Promise<void> {
    if (!session_id) return
    status = `loading`
    error_msg = ``
    confirming = null
    try {
      let r = await transport.exec(session_id, SQUEUE_ME, 20000)
      // Fall back to `-u $USER` if `--me` is unsupported on this Slurm build.
      if (r.code !== 0 && /--me|unrecognized|invalid option/i.test(r.stderr)) {
        r = await transport.exec(session_id, SQUEUE_USER, 20000)
      }
      if (r.code !== 0) {
        if (looks_missing(`${r.stderr} ${r.stdout}`)) {
          status = `unavailable`
          return
        }
        status = `error`
        error_msg = r.stderr.trim() || `squeue failed (exit ${r.code}).`
        return
      }
      jobs = parse_jobs(r.stdout)
      status = `ready`
    } catch (e: unknown) {
      status = `error`
      error_msg = e instanceof Error ? e.message : String(e)
    }
  }

  async function cancel_job(id: string): Promise<void> {
    if (!session_id || cancelling) return
    cancelling = id
    confirming = null
    error_msg = ``
    try {
      const r = await transport.exec(session_id, `scancel ${id}`, 20000)
      if (r.code !== 0) {
        status = `error`
        error_msg = r.stderr.trim() || `scancel ${id} failed (exit ${r.code}).`
      }
    } catch (e: unknown) {
      status = `error`
      error_msg = e instanceof Error ? e.message : String(e)
    } finally {
      cancelling = null
      await refresh()
    }
  }

  /** Lowercased state bucket for badge coloring. */
  function state_class(state: string): string {
    const s = state.toUpperCase()
    if (s === `RUNNING`) return `running`
    if (s === `PENDING`) return `pending`
    return `other`
  }

  const running_count = $derived(
    jobs.filter((j) => j.state.toUpperCase() === `RUNNING`).length,
  )
  const pending_count = $derived(
    jobs.filter((j) => j.state.toUpperCase() === `PENDING`).length,
  )

  // Initial load when a session is available.
  $effect(() => {
    if (session_id && status === `init`) {
      refresh()
    }
  })
</script>

<div class="mobile-jobs">
  <header class="mj-header">
    <div class="mj-summary">
      <span class="mj-title">SLURM jobs</span>
      {#if status === `ready` && jobs.length > 0}
        <span class="mj-counts">
          <span class="mj-count running">{running_count} running</span>
          <span class="mj-count pending">{pending_count} pending</span>
        </span>
      {/if}
    </div>
    <button
      type="button"
      class="mj-refresh"
      onclick={refresh}
      disabled={status === `loading`}
    >
      {status === `loading` ? `Refreshing…` : `Refresh`}
    </button>
  </header>

  <div class="mj-body">
    {#if status === `init` || status === `loading`}
      <div class="mj-note">Loading jobs…</div>
    {:else if status === `unavailable`}
      <div class="mj-note">SLURM not available on this host.</div>
    {:else if status === `error`}
      <div class="mj-error" role="alert">{error_msg}</div>
    {:else if jobs.length === 0}
      <div class="mj-note">No active jobs.</div>
    {:else}
      <div class="mj-list">
        {#each jobs as job (job.id)}
          <div class="mj-card">
            <div class="mj-card-top">
              <div class="mj-id-name">
                <span class="mj-job-id">{job.id}</span>
                {#if job.name}
                  <span class="mj-job-name">{job.name}</span>
                {/if}
              </div>
              <span class="mj-badge {state_class(job.state)}">{job.state}</span>
            </div>

            <div class="mj-meta">
              <span class="mj-meta-item">
                <span class="mj-meta-label">Time</span>
                <span class="mj-meta-val">{job.time} / {job.timeLimit}</span>
              </span>
              <span class="mj-meta-item">
                <span class="mj-meta-label">Nodes</span>
                <span class="mj-meta-val">{job.nodes}</span>
              </span>
            </div>

            {#if state_class(job.state) === `pending` && job.reason}
              <div class="mj-reason">Reason: {job.reason}</div>
            {/if}

            <div class="mj-actions">
              {#if confirming === job.id}
                <span class="mj-confirm-text">Cancel job {job.id}?</span>
                <button
                  type="button"
                  class="mj-btn mj-btn-danger"
                  disabled={cancelling === job.id}
                  onclick={() => cancel_job(job.id)}
                >
                  {cancelling === job.id ? `Cancelling…` : `Confirm`}
                </button>
                <button
                  type="button"
                  class="mj-btn mj-btn-ghost"
                  onclick={() => (confirming = null)}
                >
                  Keep
                </button>
              {:else}
                <button
                  type="button"
                  class="mj-btn mj-btn-danger"
                  disabled={cancelling === job.id}
                  onclick={() => (confirming = job.id)}
                >
                  Cancel
                </button>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .mobile-jobs {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--page-bg, #0e1117);
  }
  .mj-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-shrink: 0;
    padding: 10px 14px;
    background: rgba(0, 0, 0, 0.3);
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .mj-summary {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .mj-title {
    font-size: 0.9em;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
  }
  .mj-counts {
    display: flex;
    gap: 8px;
    font-size: 0.72em;
  }
  .mj-count {
    color: var(--text-color-muted, #94a3b8);
  }
  .mj-count.running {
    color: #4ade80;
  }
  .mj-count.pending {
    color: #fbbf24;
  }
  .mj-refresh {
    flex-shrink: 0;
    min-height: 36px;
    padding: 0 14px;
    font-size: 14px;
    color: var(--accent-color, #3b82f6);
    background: rgba(59, 130, 246, 0.1);
    border: 1px solid rgba(59, 130, 246, 0.3);
    border-radius: 8px;
    cursor: pointer;
  }
  .mj-refresh:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .mj-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px;
    padding-bottom: max(14px, env(safe-area-inset-bottom));
    box-sizing: border-box;
  }
  .mj-note {
    padding: 12px;
    font-size: 0.9em;
    color: var(--text-color-muted, #94a3b8);
    text-align: center;
  }
  .mj-error {
    font-size: 0.85em;
    color: #ff6b6b;
    background: rgba(255, 107, 107, 0.1);
    border: 1px solid rgba(255, 107, 107, 0.3);
    border-radius: 8px;
    padding: 10px 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .mj-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .mj-card {
    background: var(--surface-bg, #1a1a2e);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 12px;
    padding: 14px;
  }
  .mj-card-top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }
  .mj-id-name {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .mj-job-id {
    font-size: 15px;
    font-weight: 600;
    color: var(--text-color, #e0e0e0);
  }
  .mj-job-name {
    font-size: 0.82em;
    color: var(--text-color-muted, #94a3b8);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mj-badge {
    flex-shrink: 0;
    font-size: 0.72em;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 999px;
    color: var(--text-color-muted, #94a3b8);
    background: rgba(255, 255, 255, 0.08);
  }
  .mj-badge.running {
    color: #4ade80;
    background: rgba(74, 222, 128, 0.12);
  }
  .mj-badge.pending {
    color: #fbbf24;
    background: rgba(251, 191, 36, 0.12);
  }
  .mj-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    margin-top: 12px;
  }
  .mj-meta-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .mj-meta-label {
    font-size: 0.7em;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-color-muted, #94a3b8);
  }
  .mj-meta-val {
    font-size: 0.9em;
    color: var(--text-color, #e0e0e0);
    font-variant-numeric: tabular-nums;
  }
  .mj-reason {
    margin-top: 10px;
    font-size: 0.8em;
    color: var(--text-color-muted, #94a3b8);
    word-break: break-word;
  }
  .mj-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 14px;
  }
  .mj-confirm-text {
    flex: 1;
    min-width: 0;
    font-size: 0.82em;
    color: var(--text-color, #e0e0e0);
  }
  .mj-btn {
    min-height: 40px;
    padding: 0 16px;
    font-size: 14px;
    font-weight: 600;
    border-radius: 8px;
    cursor: pointer;
  }
  .mj-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .mj-btn-danger {
    color: #ff6b6b;
    background: rgba(255, 107, 107, 0.1);
    border: 1px solid rgba(255, 107, 107, 0.3);
  }
  .mj-btn-ghost {
    color: var(--text-color-muted, #94a3b8);
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.14);
  }
</style>

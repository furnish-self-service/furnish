<script lang="ts">
  // The steps of installing/updating or uninstalling an app: a preview (DSC what-if, changes
  // nothing) or, during and after a run, live progress of those same steps.
  import * as api from './api';
  import type { AppInfo, Step } from './api';
  import { store, type RunStep } from './state.svelte';
  import { describeCondition } from './conditions';

  let { app }: { app: AppInfo } = $props();

  const status = $derived(store.status[app.key]);
  const installed = $derived(status === 'installed' || status === 'needsUpdate');
  const run = $derived(store.runs[app.key]);
  const running = $derived(!!store.busy[app.key]);

  type Mode = 'install' | 'uninstall' | 'run';
  let picked = $state<Mode | null>(null);
  // While an operation runs, always follow it; otherwise show what the user picked, or the
  // preview of the app's primary action.
  const mode: Mode = $derived(running ? 'run' : (picked ?? (status === 'installed' ? 'uninstall' : 'install')));
  const installLabel = $derived(status === 'needsUpdate' ? 'Update' : 'Install');

  // Once a run starts, keep showing it after it ends, so its results stay visible.
  $effect(() => {
    if (running) picked = 'run';
  });

  let preview = $state<Step[] | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  // Load the preview when it is shown, and again after the app's state changed.
  $effect(() => {
    if (mode === 'run' || running) return;
    const uninstall = mode === 'uninstall';
    status; // dependency: refresh after install/uninstall
    loading = true;
    error = null;
    api
      .plan(app.key, uninstall)
      .then((s) => (preview = s))
      .catch((e) => ((error = String(e)), (preview = null)))
      .finally(() => (loading = false));
  });

  // A clock for the running step's duration.
  let now = $state(Date.now());
  $effect(() => {
    if (!running) return;
    const timer = setInterval(() => (now = Date.now()), 200);
    return () => clearInterval(timer);
  });

  /** Rows to show: preview steps, or the run's steps with their progress. */
  const rows: RunStep[] = $derived(
    mode === 'run' ? (run?.steps ?? []) : (preview ?? []).map((step) => ({ step, state: 'pending' as const })),
  );

  const labels: Record<Step['action'], string> = {
    install: 'Install',
    remove: 'Remove',
    change: 'Change',
    nothing: 'No change',
    skipped: 'Skipped',
  };
  const kindVerb = { install: 'Installing', update: 'Updating', uninstall: 'Uninstalling' } as const;
  const kindDone = { install: 'Installed', update: 'Updated', uninstall: 'Uninstalled' } as const;
  const kindNoun = { install: 'Installation', update: 'Update', uninstall: 'Uninstall' } as const;

  const active = $derived(rows.filter((r) => r.step.action !== 'skipped'));
  const changing = $derived(active.filter((r) => r.step.action !== 'nothing').length);
  const skippedCount = $derived(rows.length - active.length);
  const previewVerb = $derived(mode === 'uninstall' ? 'Uninstalling' : status === 'needsUpdate' ? 'Updating' : 'Installing');
  const finished = $derived(rows.filter((r) => r.state !== 'pending' && r.state !== 'running').length);
  const failedRow = $derived(rows.find((r) => r.state === 'failed'));
  const total = $derived(run ? (run.finishedAt ?? now) - run.startedAt : 0);

  /** "Run as administrator: cmd" → who + command, rendered as a command line. */
  function command(detail: string) {
    const m = detail.match(/^Run(?: as (administrator|your user))?: (.+)$/);
    return m ? { who: m[1] === 'administrator' ? 'Runs as administrator' : m[1] ? 'Runs as you' : 'Runs', cmd: m[2] } : null;
  }
  /** Skipped steps carry "Not on this system: <condition>". */
  function skipReason(detail: string) {
    const condition = detail.match(/^Not on this system: (.+)$/)?.[1];
    return condition ? { text: describeCondition(condition) ?? 'Its condition is false here', condition } : null;
  }
  const show = (v: unknown) => (v === null || v === undefined ? '—' : typeof v === 'string' ? v : JSON.stringify(v));
  const seconds = (ms: number) => (ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`);
  const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });

  function result(row: RunStep): string {
    const o = row.outcome;
    if (!o) return '';
    if (o.cancelled) return 'Cancelled';
    if (o.error) return 'Failed';
    return { install: 'Installed', remove: 'Removed', change: 'Changed', nothing: 'Already as described', skipped: 'Skipped' }[o.action];
  }
</script>

<div class="plan">
  <div class="switch" role="radiogroup" aria-label="Show">
    <button role="radio" aria-checked={mode === 'install'} class:on={mode === 'install'} disabled={running} onclick={() => (picked = 'install')}>
      {installLabel}
    </button>
    <button role="radio" aria-checked={mode === 'uninstall'} class:on={mode === 'uninstall'} disabled={running || !installed} onclick={() => (picked = 'uninstall')}>
      Uninstall
    </button>
    {#if run}
      <button role="radio" aria-checked={mode === 'run'} class:on={mode === 'run'} onclick={() => (picked = 'run')}>
        {running ? 'Running' : 'Last run'}
      </button>
    {/if}
  </div>

  {#if mode === 'run' && run}
    <p class="summary" aria-live="polite">
      {#if running}
        <span class="pulse" aria-hidden="true"></span>
        {kindVerb[run.kind]} {app.id}: <strong>step {Math.min(finished + 1, rows.length || 1)} of {rows.length || '…'}</strong>
        <span class="muted">· {seconds(total)}</span>
        {#if run.stopping === 'abort'}<span class="stopping">Stopping after this step…</span>
        {:else if run.stopping === 'force'}<span class="stopping">Interrupting this step…</span>{/if}
      {:else if run.cancelled}
        <strong>{kindNoun[run.kind]} of {app.id} cancelled</strong> after {seconds(total)}.
        <span class="muted">Steps already done stay done.</span>
      {:else if failedRow}
        <strong class="failed-text">{kindVerb[run.kind]} {app.id} failed</strong> at “{failedRow.step.name}” after {seconds(total)}.
      {:else}
        <strong>{kindDone[run.kind]} {app.id}</strong> in {seconds(total)}
        <span class="muted">· started {clock(run.startedAt)}</span>
      {/if}
    </p>
  {:else if error}
    <p class="error">Couldn't prepare the preview: {error}</p>
  {:else if !preview}
    <p class="muted">Preparing the preview…</p>
  {:else}
    <p class="summary" class:stale={loading}>
      {previewVerb} {app.id} would
      {#if changing === 0}
        <strong>change nothing</strong>: everything is already as described.
      {:else}
        make <strong>{changing} change{changing === 1 ? '' : 's'}</strong>.
      {/if}
      {#if skippedCount}<span class="muted">{skippedCount} step{skippedCount === 1 ? ' does' : 's do'} not apply to this system.</span>{/if}
    </p>
  {/if}

  <ol class="steps" class:live={mode === 'run'}>
    {#each rows as row, i (i)}
      {@const step = row.step}
      <li class="{step.action} {row.state}">
        {#if mode === 'run'}
          <span class="marker" title={row.state} aria-label={row.state}>
            {#if row.state === 'running'}<span class="spinner"></span>
            {:else if row.state === 'done'}✓
            {:else if row.state === 'failed'}✕
            {:else if row.state === 'skipped'}–
            {:else if row.state === 'cancelled'}⊘
            {:else}{i + 1}{/if}
          </span>
        {/if}
        <span class="badge">{labels[step.action]}</span>
        <div class="body">
          <div class="title">
            <span class="name">{step.name}</span>
            <span class="type">{step.type}</span>
          </div>
          {#each step.details as detail}
            {@const c = command(detail)}
            {@const skip = skipReason(detail)}
            {#if skip}
              <p class="detail">Not for this system: {skip.text}. <span class="type">{skip.condition}</span></p>
            {:else if c}
              <div class="command">
                <span class="who">{c.who}</span>
                <code>{c.cmd}</code>
              </div>
            {:else}
              <p class="detail">{detail}</p>
            {/if}
          {/each}
          {#each row.outcome?.changes ?? step.changes as change}
            <p class="diff"><span>{change.property}</span> {show(change.before)} <span aria-label="becomes">→</span> {show(change.after)}</p>
          {/each}
          {#if row.outcome?.error}
            <pre class="failure">{row.outcome.error}</pre>
          {/if}
        </div>
        {#if mode === 'run' && row.state === 'pending' && !running}
          <div class="timing"><span class="muted">Not run</span></div>
        {:else if mode === 'run' && row.state !== 'pending' && row.state !== 'skipped'}
          <div class="timing">
            <span class="result">{row.state === 'running' ? 'Running' : result(row)}</span>
            <span class="muted">
              {row.startedAt ? clock(row.startedAt) : ''} ·
              {seconds(row.durationMs ?? (row.startedAt ? now - row.startedAt : 0))}
            </span>
          </div>
        {/if}
      </li>
    {/each}
  </ol>
  {#if mode !== 'run' && preview}
    <p class="note">This is a preview. Nothing has been changed on this computer.</p>
  {/if}
</div>

<style>
  .plan {
    margin-top: 0.75rem;
  }
  .switch {
    display: inline-flex;
    padding: 3px;
    border-radius: 999px;
    background: var(--hover);
  }
  .switch button {
    padding: 0.35rem 1rem;
    border: 0;
    border-radius: 999px;
    background: none;
    color: var(--ink-soft);
    font: 600 0.82rem var(--font-body);
    cursor: pointer;
  }
  .switch button.on {
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow);
    opacity: 1;
  }
  .summary {
    margin: 1rem 0 0.75rem;
    line-height: 1.5;
    transition: opacity 0.15s;
  }
  .stale {
    opacity: 0.5;
  }
  .muted {
    color: var(--muted);
  }
  .error,
  .failed-text {
    color: var(--danger);
  }
  .pulse {
    display: inline-block;
    width: 0.55rem;
    height: 0.55rem;
    margin-right: 0.35rem;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% { opacity: 0.25; }
  }
  .steps {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.5rem;
  }
  .steps li {
    display: flex;
    gap: 0.9rem;
    align-items: flex-start;
    padding: 0.85rem 1rem;
    border: 1px solid var(--line);
    border-left: 4px solid var(--tone);
    border-radius: 12px;
    background: var(--surface);
    --tone: var(--line-strong);
    --tone-soft: var(--hover);
    --tone-ink: var(--ink-soft);
    transition: opacity 0.2s, box-shadow 0.2s;
  }
  .steps li.install {
    --tone: var(--accent);
    --tone-soft: var(--accent-soft);
    --tone-ink: var(--accent-ink);
  }
  .steps li.remove,
  .steps li.failed {
    --tone: var(--danger);
    --tone-soft: var(--danger-soft);
    --tone-ink: var(--danger);
  }
  .steps li.change {
    --tone: var(--highlight);
    --tone-soft: var(--highlight-soft);
    --tone-ink: var(--highlight-ink);
  }
  .steps li.skipped {
    background: transparent;
    border-style: dashed;
    border-left-style: solid;
    opacity: 0.75;
  }
  /* Live view: upcoming steps recede, the current one stands out. */
  .live li.pending {
    opacity: 0.55;
  }
  .live li.running {
    box-shadow: 0 0 0 2px var(--tone-soft), var(--shadow-raised);
  }
  .marker {
    flex: none;
    display: grid;
    place-items: center;
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 50%;
    font: 700 0.8rem var(--font-body);
    color: var(--muted);
    border: 2px solid var(--line-strong);
  }
  .done .marker {
    color: var(--on-accent);
    background: var(--accent);
    border-color: var(--accent);
  }
  .failed .marker {
    color: var(--surface);
    background: var(--danger);
    border-color: var(--danger);
  }
  .running .marker {
    border-color: var(--accent);
  }
  .cancelled .marker {
    color: var(--surface);
    background: var(--muted);
    border-color: var(--muted);
  }
  .steps li.cancelled {
    --tone: var(--muted);
    --tone-soft: var(--hover);
    --tone-ink: var(--ink-soft);
  }
  .stopping {
    margin-left: 0.5rem;
    font-weight: 650;
    color: var(--danger);
  }
  .spinner {
    width: 0.8rem;
    height: 0.8rem;
    border: 2px solid var(--accent);
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner,
    .pulse {
      animation: none;
    }
  }
  .badge {
    flex: none;
    min-width: 5.6rem;
    padding: 0.2rem 0.6rem;
    border-radius: 999px;
    text-align: center;
    font: 700 0.72rem var(--font-body);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--tone-ink);
    background: var(--tone-soft);
  }
  .body {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 0.35rem;
  }
  .title {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem 0.75rem;
    align-items: baseline;
  }
  .name {
    font-weight: 650;
  }
  .type {
    font: 0.75rem var(--font-mono);
    color: var(--muted);
  }
  .detail {
    margin: 0;
    font-size: 0.9rem;
    color: var(--ink-soft);
  }
  .command {
    display: grid;
    gap: 0.25rem;
    padding: 0.5rem 0.7rem;
    border-radius: 8px;
    background: #0e1d21;
  }
  .who {
    font: 700 0.68rem var(--font-body);
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: #f6c453;
  }
  code {
    font: 0.8rem/1.45 var(--font-mono);
    color: #d5e5e3;
    word-break: break-all;
  }
  .diff {
    margin: 0;
    font: 0.82rem var(--font-mono);
    color: var(--ink-soft);
  }
  .diff span:first-child {
    font-weight: 700;
  }
  .failure {
    margin: 0;
    padding: 0.5rem 0.7rem;
    border-radius: 8px;
    background: var(--danger-soft);
    color: var(--danger);
    font: 0.78rem/1.4 var(--font-mono);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .timing {
    flex: none;
    display: grid;
    justify-items: end;
    gap: 0.15rem;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
  .result {
    font-weight: 650;
    color: var(--tone-ink);
  }
  .note {
    margin: 0.75rem 0 0;
    font-size: 0.8rem;
    color: var(--muted);
  }
</style>

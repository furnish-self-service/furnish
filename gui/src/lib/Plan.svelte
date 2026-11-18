<script lang="ts">
  // The steps of installing/updating or uninstalling an app: a preview (DSC what-if, changes
  // nothing) or, during and after a run, live progress of those same steps.
  import * as api from './api';
  import type { AppInfo, Step } from './api';
  import type { Run, RunStep } from './activity-history';
  import { store, appName, externalNote } from './state.svelte';
  import { stepProse } from './step-prose';
  import { describeCondition } from './conditions';
  import { t, terror, tparts, parts, formatTime, formatDuration, formatDate } from './i18n.svelte';
  import { dictionaries, type Key } from './i18n.ts';

  // Activity supplies the actual operation, even if the app has since left the catalog.
  let { app, operation }: { app: AppInfo; operation?: never } | { app?: never; operation: Run } = $props();

  const status = $derived(app && store.status[app.key]);
  const installed = $derived(status === 'installed' || status === 'needsUpdate');
  const run = $derived(operation ?? (app && store.runs[app.key]));
  const name = $derived(app ? appName(app) : operation!.name);
  const running = $derived(!!run && run.finishedAt === undefined);

  type Mode = 'install' | 'uninstall' | 'run';
  let picked = $state<Mode | null>(null);
  // While an operation runs, always follow it; otherwise show what the user picked, or the
  // preview of the app's primary action.
  const unremovable = $derived(app && !!store.unremovable[app.key]);
  const mode: Mode = $derived(operation || running ? 'run' : (picked ?? (status === 'installed' && !unremovable ? 'uninstall' : 'install')));
  const installLabel = $derived(status === 'needsUpdate' ? t('common.update') : t('common.install'));

  // Once a run starts, keep showing it after it ends, so its results stay visible.
  $effect(() => {
    if (running) picked = 'run';
  });

  let preview = $state<Step[] | null>(null);
  let error = $state<string | null>(null);

  // Load the preview when it is shown, and again after the app's state changed.
  $effect(() => {
    if (!app || mode === 'run' || running) return;
    const uninstall = mode === 'uninstall';
    status; // dependency: refresh after install/uninstall
    if (uninstall && unremovable) return; // nothing to preview, see the note below
    let current = true;
    preview = null;
    error = null;
    api.plan(app.key, uninstall).then(
      (steps) => { if (current) preview = steps; },
      (reason) => { if (current) error = String(reason); },
    );
    return () => { current = false; };
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
    mode === 'run'
      ? (run?.steps ?? [])
      : mode === 'uninstall' && unremovable
        ? []
        : (preview ?? []).map((step) => ({ step, state: 'pending' as const })),
  );


  const active = $derived(rows.filter((r) => r.step.action !== 'skipped'));
  const changing = $derived(active.filter((r) => r.step.action !== 'nothing').length);
  const skippedCount = $derived(rows.length - active.length);
  const previewKind = $derived(mode === 'uninstall' ? 'uninstall' : status === 'needsUpdate' ? 'update' : 'install');
  const finished = $derived(rows.filter((r) => r.state !== 'pending' && r.state !== 'running').length);
  const failedRow = $derived(rows.slice(run?.phaseStart ?? 0).find((r) => r.state === 'failed'));
  const total = $derived(run ? (run.finishedAt ?? now) - run.startedAt : 0);

  /** "Run as administrator: cmd" → who + command, rendered as a command line. */
  function command(detail: string) {
    const m = detail.match(/^Run(?: as (administrator|your user))?: (.+)$/);
    return m
      ? { who: m[1] === 'administrator' ? t('plan.runsAsAdmin') : m[1] ? t('plan.runsAsYou') : t('plan.runs'), cmd: m[2] }
      : null;
  }
  /** Skipped steps carry "Not on this system: <condition>". */
  function skipReason(detail: string) {
    const prefix = 'Not on this system: ';
    const condition = detail.startsWith(prefix) && detail.slice(prefix.length);
    return condition ? { text: describeCondition(condition) ?? t('plan.conditionFalse'), condition } : null;
  }
  /** A Furnish/NixOS step with onlyIfSetUp where its configuration isn't set up:
   *  "Not needed here: <reason>", with the reasons of src/nixos_apps.rs. */
  const notNeededReasons: Record<string, Key> = {
    "Nix isn't installed on this computer": 'plan.notNeeded.nix',
    "Home Manager isn't set up for you": 'plan.notNeeded.home',
    "the NixOS configuration isn't set up for furnish": 'plan.notNeeded.system',
  };
  function notNeeded(detail: string) {
    const reason = detail.match(/^Not needed here: (.+)$/)?.[1];
    return reason ? (notNeededReasons[reason] ? t(notNeededReasons[reason]) : reason) : null;
  }
  type Note = NonNullable<Step['notes']>[number];
  /** furnish's configuration resources describe their steps as notes to translate; a note of a
   *  newer furnish that this interface doesn't know shows the English details instead. */
  const known = (n: Note) => `plan.note.${n.code}` in dictionaries.en || n.code === 'run';
  /** Furnish/Command's notes name a command: it gets its own box, as a command that runs does,
   *  and the sentence below says what the test found. */
  const boxed = (n: Note) => typeof n.params.command === 'string' && (n.code.startsWith('test.') || n.code === 'command.keep');
  function noteParts(n: Note) {
    const p = n.params as Record<string, string | number>;
    return tparts(`plan.note.${n.code}` as Key, n.code === 'file.create' ? { ...p, n: p.lines } : p);
  }
  /** A file step's diff (resources/diff.rs): hunks of ' kept', '-removed', '+added' lines, and
   *  '\\' marks after a line that ends its text without a line break. */
  type Diff = { hunks: { old: number; new: number; lines: string[] }[]; more: number };
  type DiffRow = { old: number | null; new: number | null; sign: ' ' | '+' | '-' | 'eol'; text: string };
  /** The hunks as rows with their line numbers in the old and the new file. */
  function diffHunks(n: Note): DiffRow[][] {
    const d = n.params.diff as Diff | undefined;
    return (d?.hunks ?? []).map((h) => {
      let [o, nw] = [h.old, h.new];
      return h.lines.map((l) => {
        if (l[0] === '\\') {
          // Which text ends here without a line break: the old one, the new one or both.
          const text = l[1] === '-' ? t('plan.diff.noNewlineBefore') : l[1] === '+' ? t('plan.diff.noNewlineAfter') : t('plan.diff.noNewline');
          return { old: null, new: null, sign: 'eol', text } as DiffRow;
        }
        const sign = l[0] === '+' || l[0] === '-' ? l[0] : ' ';
        const row = { old: sign === '+' ? null : o, new: sign === '-' ? null : nw, sign, text: l.slice(1) } as DiffRow;
        if (sign !== '+') o++;
        if (sign !== '-') nw++;
        return row;
      });
    });
  }
  const diffMore = (n: Note) => (n.params.diff as Diff | undefined)?.more ?? 0;
  /** "+3 −1", unless the note's sentence already has the counts. */
  const showCounts = (n: Note) => typeof n.params.added === 'number' && n.code !== 'file.write' && n.code !== 'file.create';
  /** The properties a step changes, before and after. Furnish/Value's value is left out when the
   *  diff shows it in the file. */
  function changes(row: RunStep) {
    const all = row.outcome?.changes ?? row.step.changes;
    const diffed = row.step.type === 'Furnish/Value' && row.step.notes?.some((n) => n.params.diff);
    return diffed ? all.filter((c) => c.property !== 'value') : all;
  }
  const show = (v: unknown) => (v === null || v === undefined ? '—' : typeof v === 'string' ? v : JSON.stringify(v));
  const seconds = formatDuration;
  const clock = (ms: number) => formatTime(ms, true);

  function result(row: RunStep): string {
    const o = row.outcome;
    if (!o) return '';
    if (o.cancelled) return t('plan.result.cancelled');
    if (o.error) return t('plan.result.failed');
    return t(`plan.result.${o.action}`);
  }
</script>

{#snippet marked(text: string, strongClass = '')}{#each parts(text) as p}{#if p.tag}<strong class={strongClass}>{p.text}</strong>{:else}{p.text}{/if}{/each}{/snippet}

{#snippet prose(text: string)}{#each stepProse(text) as part}{#if part.code}<code class="inline">{part.text}</code>{:else}{part.text}{/if}{/each}{/snippet}

{#snippet sentence(n: Note)}{#each noteParts(n) as part}{#if part.tag}<code class="inline">{part.text}</code>{:else}{@render prose(part.text)}{/if}{/each}{#if showCounts(n)}<span class="counts" title={t('plan.diff.counts', { added: n.params.added as number, removed: n.params.removed as number })}>{#if n.params.added}<span class="plus">+{n.params.added}</span>{/if}{#if n.params.added && n.params.removed}{' '}{/if}{#if n.params.removed}<span class="minus">−{n.params.removed}</span>{/if}</span>{/if}{/snippet}

{#snippet diffTable(n: Note)}
  <div class="difftable">
    {#each diffHunks(n) as hunk, h}
      {#if h > 0}<div class="sep" aria-hidden="true"><span>⋯</span></div>{/if}
      {#each hunk as r}
        {#if r.sign === 'eol'}
          <div class="row eol"><span></span><span></span><span></span><span class="text">{r.text}</span></div>
        {:else}
        <div class="row" class:add={r.sign === '+'} class:del={r.sign === '-'}>
          <span class="num" aria-hidden="true">{r.old ?? ''}</span>
          <span class="num" aria-hidden="true">{r.new ?? ''}</span>
          <span class="sign">{r.sign === '-' ? '−' : r.sign}</span>
          <span class="text">{r.text}</span>
        </div>
        {/if}
      {/each}
    {/each}
    {#if diffMore(n)}<div class="more">{t('plan.diff.more', { n: diffMore(n) })}</div>{/if}
  </div>
{/snippet}

<div class="plan">
  {#if app}
    <div class="switch" role="radiogroup" aria-label={t('plan.show')}>
      <button role="radio" aria-checked={mode === 'install'} class:on={mode === 'install'} disabled={running} onclick={() => (picked = 'install')}>
        {installLabel}
      </button>
      <button role="radio" aria-checked={mode === 'uninstall'} class:on={mode === 'uninstall'} disabled={running || !installed} onclick={() => (picked = 'uninstall')}>
        {t('common.uninstall')}
      </button>
      {#if run}
        <button role="radio" aria-checked={mode === 'run'} class:on={mode === 'run'} onclick={() => (picked = 'run')}>
          {running ? t('plan.running') : t('plan.lastRun')}
        </button>
      {/if}
    </div>
  {/if}

  {#if mode === 'run' && run}
    <p class="summary" aria-live="polite">
      {#if running}
        <span class="pulse" aria-hidden="true"></span>
        {@render marked(
          t('plan.runningSummary', {
            what: t(`plan.running.${run.kind}`, { app: name }),
            step: t('run.stepOf', { n: Math.min(finished + 1, rows.length || 1), total: rows.length || '…' }),
          }),
        )}
        <span class="muted">· {seconds(total)}</span>
        {#if run.stopping === 'abort'}<span class="stopping">{t('plan.stoppingAfter')}</span>
        {:else if run.stopping === 'force'}<span class="stopping">{t('plan.interrupting')}</span>{/if}
      {:else if run.result === 'cancelled'}
        {@render marked(t('plan.cancelledAfter', { what: t(`run.cancelled.${run.kind}`, { app: name }), time: seconds(total) }))}
        <span class="muted">{t('plan.doneStepsStay')}</span>
      {:else if run.result === 'fallback'}
        {t('toast.fellBack', { app: name, date: formatDate(run.fallbackDate!) })}
      {:else if run.result === 'failed' && failedRow}
        {@render marked(
          t('plan.failedAt', { what: t(`plan.failed.${run.kind}`, { app: name }), step: failedRow.step.name, time: seconds(total) }),
          'failed-text',
        )}
      {:else if run.result === 'failed'}
        <span class="failed-text">{t(`run.failed.${run.kind}`, { app: name })}</span>
      {:else}
        {@render marked(t('plan.doneIn', { what: t(`run.done.${run.kind}`, { app: name }), time: seconds(total) }))}
        <span class="muted">· {t('plan.started', { time: clock(run.startedAt) })}</span>
      {/if}
    </p>
    {#if run.result === 'failed' && run.error && !failedRow?.outcome?.error}
      <pre class="failure">{terror(run.error)}</pre>
    {/if}
  {:else if app && mode === 'uninstall' && unremovable}
    <p class="muted">
      {store.external[app.key]
        ? externalNote(store.external[app.key])
        : t('app.cantUninstall', { collection: app.collection })}
    </p>
  {:else if error}
    <p class="error">{t('plan.previewError', { error })}</p>
  {:else if !preview}
    <p class="muted">{t('plan.preparing')}</p>
  {:else}
    <p class="summary">
      {@render marked(
        changing === 0
          ? t('plan.changeNothing', { subject: t(`plan.subject.${previewKind}`, { app: name }) })
          : t('plan.changes', { subject: t(`plan.subject.${previewKind}`, { app: name }), n: changing }),
      )}
      {#if skippedCount}<span class="muted">{t('plan.skipped', { n: skippedCount })}</span>{/if}
    </p>
  {/if}

  <ol class="steps" class:live={mode === 'run'}>
    {#each rows as row, i (i)}
      {@const step = row.step}
      <li class="{step.action} {row.state}">
        {#if mode === 'run'}
          {@const stateLabel = row.state === 'notRun' ? t('plan.notRun') : t(`plan.state.${row.state}`)}
          <span class="marker" title={stateLabel} aria-label={stateLabel}>
            {#if row.state === 'running'}<span class="spinner"></span>
            {:else if row.state === 'done'}✓
            {:else if row.state === 'failed'}✕
            {:else if row.state === 'skipped' || row.state === 'notRun'}–
            {:else if row.state === 'cancelled'}⊘
            {:else}{i + 1}{/if}
          </span>
        {/if}
        <span class="badge">{t(`plan.action.${step.action}`)}</span>
        <div class="body">
          <div class="title">
            <span class="name">{step.name}</span>
            <span class="type">{step.type}</span>
          </div>
          {#each step.selectionNotes ?? [] as n}
            <p class="detail">{#if known(n)}{@render sentence(n)}{:else}{@render prose(n.english)}{/if}</p>
          {/each}
          {#if step.notes?.length && step.notes.every(known)}
            {#each step.notes as n}
              {#if n.code === 'run'}
                <div class="command">
                  <span class="who">{t('plan.runs')}</span>
                  <code>{n.params.command}</code>
                </div>
              {:else}
                {#if boxed(n)}
                  <div class="command">
                    <span class="who">{t(n.code === 'command.keep' ? 'plan.keeps' : 'plan.tests')}</span>
                    <code>{n.params.command}</code>
                  </div>
                {/if}
                {#if n.params.diff}
                  <details class="filediff">
                    <summary><span class="detail">{@render sentence(n)}</span></summary>
                    {@render diffTable(n)}
                  </details>
                {:else}
                  <p class="detail">{@render sentence(n)}</p>
                  {#if n.params.binary}<p class="lines-note">{t('plan.diff.binary')}</p>
                  {:else if n.params.hidden}<p class="lines-note">{t('plan.diff.hidden')}</p>{/if}
                {/if}
              {/if}
            {/each}
          {:else}
          {#each step.details as detail}
            {@const c = command(detail)}
            {@const skip = skipReason(detail)}
            {@const unneeded = notNeeded(detail)}
            {#if skip}
              <p class="detail">{t('plan.notForSystem', { reason: skip.text })}</p>
              <code class="condition">{skip.condition}</code>
            {:else if unneeded}
              <p class="detail">{t('plan.notNeeded', { reason: unneeded })}</p>
            {:else if c}
              <div class="command">
                <span class="who">{c.who}</span>
                <code>{c.cmd}</code>
              </div>
            {:else}
              <p class="detail">{@render prose(detail)}</p>
            {/if}
          {/each}
          {/if}
          {#each changes(row) as change}
            <p class="diff"><span>{change.property}</span> {show(change.before)} <span aria-label={t('plan.becomes')}>→</span> {show(change.after)}</p>
          {/each}
          {#if row.outcome?.error}
            <pre class="failure">{terror(row.outcome.error)}</pre>
          {/if}
        </div>
        {#if mode === 'run' && (row.state === 'notRun' || (row.state === 'pending' && !running))}
          <div class="timing"><span class="muted">{t('plan.notRun')}</span></div>
        {:else if mode === 'run' && row.state !== 'pending' && row.state !== 'skipped'}
          <div class="timing">
            <span class="result">{row.state === 'running' ? t('plan.running') : result(row)}</span>
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
    <p class="note">{t('plan.note')}</p>
  {/if}
</div>

<style>
  .plan {
    margin-top: 0.75rem;
    container-type: inline-size;
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
  }
  /* Live view: upcoming steps recede, the current one stands out. */
  .live li.pending, .live li.notRun {
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
    width: 6.75rem;
    box-sizing: border-box;
    overflow-wrap: anywhere;
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
    overflow-wrap: anywhere;
  }
  .type {
    font: 0.75rem var(--font-mono);
    color: var(--muted);
  }
  .detail {
    margin: 0;
    font-size: 0.9rem;
    line-height: 1.65;
    overflow-wrap: anywhere;
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
  /* Long commands, paths and URLs wrap at spaces, and inside a word only where nothing else fits. */
  code {
    font: 0.8rem/1.45 var(--font-mono);
    color: #d5e5e3;
    overflow-wrap: anywhere;
  }
  .inline,
  .condition {
    padding: 0.08em 0.34em;
    border: 1px solid var(--line);
    border-radius: 0.32em;
    background: var(--canvas);
    color: var(--ink);
    font: 0.9em/1.5 var(--font-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    -webkit-box-decoration-break: clone;
    box-decoration-break: clone;
  }
  .condition {
    justify-self: start;
    max-width: 100%;
    padding: 0.25em 0.55em;
    color: var(--ink-soft);
    font-size: 0.8rem;
  }
  /* A file step's diff: collapsed under the step's sentence. */
  /* The browser's own disclosure triangle, at the size of the app's other ones (AppDetail's
     summaries: 0.95rem), on the sentence's baseline; the whole row toggles. */
  .filediff summary {
    cursor: pointer;
    font-size: 0.95rem;
    color: var(--ink-soft);
  }
  .filediff summary:hover,
  .filediff summary:hover .detail {
    color: var(--ink);
  }
  .difftable .eol .text {
    font: italic 0.72rem var(--font-body);
    color: var(--muted);
  }
  .counts {
    margin-left: 0.5rem;
    font: 600 0.78rem var(--font-mono);
    white-space: nowrap;
  }
  .plus {
    color: var(--accent-ink);
  }
  .minus {
    color: var(--danger);
  }
  .lines-note {
    margin: 0;
    font-size: 0.8rem;
    color: var(--muted);
  }
  .difftable {
    margin: 0.4rem 0 0;
    padding: 0.3rem 0;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--canvas);
    font: 0.76rem/1.45 var(--font-mono);
    overflow: hidden;
  }
  .difftable .row {
    display: grid;
    grid-template-columns: 3.2em 3.2em 1.4em 1fr;
  }
  .difftable .num {
    padding-right: 0.6em;
    text-align: right;
    color: var(--muted);
    user-select: none;
    font-variant-numeric: tabular-nums;
  }
  .difftable .sign {
    text-align: center;
    user-select: none;
    color: var(--muted);
  }
  .difftable .text {
    padding-right: 0.6rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--ink-soft);
  }
  .difftable .add {
    background: var(--accent-soft);
  }
  .difftable .add .sign,
  .difftable .add .text {
    color: var(--accent-ink);
  }
  .difftable .del {
    background: var(--danger-soft);
  }
  .difftable .del .sign,
  .difftable .del .text {
    color: var(--danger);
  }
  .difftable .sep {
    padding: 0.1rem 0 0.1rem 7.2em;
    color: var(--muted);
    border-block: 1px dashed var(--line);
    margin: 0.2rem 0;
  }
  .difftable .more {
    padding: 0.3rem 0.6rem 0.1rem 7.8em;
    font-family: var(--font-body);
    color: var(--muted);
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
  @container (max-width: 32rem) {
    .steps li { flex-wrap: wrap; gap: 0.4rem 0.6rem; }
    .steps .body { flex-basis: 100%; order: 1; }
    .timing { margin-inline-start: auto; }
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

<script lang="ts">
  // Preview of what installing/updating or uninstalling would do (DSC what-if). Changes nothing.
  import * as api from './api';
  import type { AppInfo, Step } from './api';
  import { store } from './state.svelte';
  import { describeCondition } from './conditions';

  let { app }: { app: AppInfo } = $props();

  const status = $derived(store.status[app.key]);
  const installed = $derived(status === 'installed' || status === 'needsUpdate');
  /** Which preview is shown; follows the app's primary action until the user picks one. */
  let picked = $state<'install' | 'uninstall' | null>(null);
  const mode = $derived(picked ?? (status === 'installed' ? 'uninstall' : 'install'));
  const installLabel = $derived(status === 'needsUpdate' ? 'Update' : 'Install');

  let steps = $state<Step[] | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  // Reload when the preview kind changes or the app's state changes (e.g. after installing).
  $effect(() => {
    const uninstall = mode === 'uninstall';
    status; // dependency: refresh after install/uninstall
    if (store.busy[app.key]) return;
    loading = true;
    error = null;
    api
      .plan(app.key, uninstall)
      .then((s) => (steps = s))
      .catch((e) => ((error = String(e)), (steps = null)))
      .finally(() => (loading = false));
  });

  const labels: Record<Step['action'], string> = {
    install: 'Install',
    remove: 'Remove',
    change: 'Change',
    nothing: 'No change',
    skipped: 'Skipped',
  };

  const active = $derived(steps?.filter((s) => s.action !== 'skipped') ?? []);
  const changing = $derived(active.filter((s) => s.action !== 'nothing').length);
  const skipped = $derived((steps?.length ?? 0) - active.length);
  const verb = $derived(mode === 'uninstall' ? 'Uninstalling' : status === 'needsUpdate' ? 'Updating' : 'Installing');

  /** "Run as administrator: cmd" → who + command, rendered as a command line. */
  function command(detail: string) {
    const m = detail.match(/^Run(?: as (administrator|your user))?: (.+)$/);
    return m ? { as: m[1] === 'administrator' ? 'Administrator' : 'Your user', cmd: m[2] } : null;
  }
  /** Skipped steps carry "Not on this system: <condition>". */
  function skipReason(detail: string) {
    const condition = detail.match(/^Not on this system: (.+)$/)?.[1];
    return condition ? { text: describeCondition(condition) ?? 'Its condition is false here', condition } : null;
  }
  const show = (v: unknown) => (v === null || v === undefined ? '—' : typeof v === 'string' ? v : JSON.stringify(v));
</script>

<div class="plan">
  <div class="switch" role="radiogroup" aria-label="Preview">
    <button role="radio" aria-checked={mode === 'install'} class:on={mode === 'install'} onclick={() => (picked = 'install')}>
      {installLabel}
    </button>
    <button role="radio" aria-checked={mode === 'uninstall'} class:on={mode === 'uninstall'} disabled={!installed && mode !== 'uninstall'} onclick={() => (picked = 'uninstall')}>
      Uninstall
    </button>
  </div>

  {#if error}
    <p class="error">Couldn't prepare the preview: {error}</p>
  {:else if !steps}
    <p class="muted">Preparing the preview…</p>
  {:else}
    <p class="summary" class:stale={loading}>
      {verb} {app.id} would
      {#if changing === 0}
        <strong>change nothing</strong>: everything is already as described.
      {:else}
        make <strong>{changing} change{changing === 1 ? '' : 's'}</strong>.
      {/if}
      {#if skipped}<span class="muted">{skipped} step{skipped === 1 ? ' does' : 's do'} not apply to this system.</span>{/if}
    </p>

    <ol class="steps">
      {#each steps as step, i (i)}
        <li class={step.action}>
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
                  <span class="who">{c.as === 'Administrator' ? 'Runs as administrator' : c.as === 'Your user' ? 'Runs as you' : 'Runs'}</span>
                  <code>{c.cmd}</code>
                </div>
              {:else}
                <p class="detail">{detail}</p>
              {/if}
            {/each}
            {#each step.changes as change}
              <p class="diff"><span>{change.property}</span> {show(change.before)} <span aria-label="becomes">→</span> {show(change.after)}</p>
            {/each}
          </div>
        </li>
      {/each}
    </ol>
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
  .error {
    color: var(--danger);
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
  }
  .steps li.install {
    --tone: var(--accent);
    --tone-soft: var(--accent-soft);
    --tone-ink: var(--accent-ink);
  }
  .steps li.remove {
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
  .note {
    margin: 0.75rem 0 0;
    font-size: 0.8rem;
    color: var(--muted);
  }
</style>

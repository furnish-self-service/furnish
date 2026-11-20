<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Action } from './api';
  import type { RunStep } from './activity-history';
  import { t } from './i18n.svelte';

  let { name, type, action, state, index, children, timing }: {
    name: string; type?: string; action?: Action; state?: RunStep['state']; index: number;
    children?: Snippet; timing?: Snippet;
  } = $props();
  const label = $derived(state === 'notRun' ? t('plan.notRun') : state && t(`plan.state.${state}`));
</script>

<li class="{action ?? ''} {state ?? ''}" class:live={!!state}>
  {#if state}
    <span class="marker" title={label} aria-label={label}>
      {#if state === 'done'}✓
      {:else if state === 'failed'}✕
      {:else if state === 'skipped' || state === 'notRun'}–
      {:else if state === 'cancelled'}⊘
      {:else}{index + 1}{/if}
    </span>
  {/if}
  {#if action && !state}<span class="badge">{t(`plan.action.${action}`)}</span>{/if}
  <div class="body">
    <div class="title">
      <span class="name">{name}</span>
      {#if type}<span class="type">{type}</span>{/if}
    </div>
    {#if timing && state}<div class="timing recorded">{@render timing()}</div>{/if}
    {#if action && state}<span class="planned">{t('plan.planned', { action: t(`plan.action.${action}`) })}</span>{/if}
    {@render children?.()}
  </div>
  {#if timing && !state}<div class="timing">{@render timing()}</div>{/if}
</li>

<style>
  li {
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
  li.install:not(.live), li.live.done {
    --tone: var(--accent);
    --tone-soft: var(--accent-soft);
    --tone-ink: var(--accent-ink);
  }
  li.remove:not(.live),
  li.failed {
    --tone: var(--danger);
    --tone-soft: var(--danger-soft);
    --tone-ink: var(--danger);
  }
  li.change:not(.live) {
    --tone: var(--highlight);
    --tone-soft: var(--highlight-soft);
    --tone-ink: var(--highlight-ink);
  }
  li.skipped {
    background: transparent;
    border-style: dashed;
    border-left-style: solid;
  }
  /* Live view: upcoming steps recede, the current one stands out. */
  li.live.pending, li.live.notRun {
    opacity: 0.55;
  }
  li.live.running {
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
    color: var(--accent-ink);
    background: var(--accent-soft);
    border-color: transparent;
  }
  .cancelled .marker {
    color: var(--surface);
    background: var(--muted);
    border-color: var(--muted);
  }
  li.cancelled {
    --tone: var(--muted);
    --tone-soft: var(--hover);
    --tone-ink: var(--ink-soft);
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
  .timing {
    flex: none;
    display: grid;
    justify-items: end;
    gap: 0.15rem;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
  @container (max-width: 32rem) {
    li { flex-wrap: wrap; gap: 0.4rem 0.6rem; }
    .body { flex-basis: 100%; order: 1; }
    .timing { margin-inline-start: auto; }
  }
  .timing.recorded { display: flex; align-items: baseline; gap: .65rem; margin: 0; flex-wrap: wrap; }
  .planned { font-size: .78rem; color: var(--muted); }
  @container (max-width: 32rem) {
    .live .body { flex: 1; order: 0; }
  }
  .timing:empty { display: none; }
  .timing :global(strong) { font-weight: 650; color: var(--tone-ink); }
</style>

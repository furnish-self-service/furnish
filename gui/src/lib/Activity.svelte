<script lang="ts">
  // What is running now and what is queued after it.
  import { store, dequeue, clearQueue } from './state.svelte';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';
  import { t } from './i18n.svelte';

  let { onopen }: { onopen: (key: string) => void } = $props();

  const runningKey = $derived(Object.keys(store.busy)[0]);
  const running = $derived(store.apps.find((a) => a.key === runningKey));
  const run = $derived(runningKey ? store.runs[runningKey] : undefined);
  const done = $derived(run?.steps.filter((s) => s.state !== 'pending' && s.state !== 'running').length ?? 0);
  const queued = $derived(
    store.queue.map((q) => ({ ...q, app: store.apps.find((a) => a.key === q.key) })).filter((q) => q.app),
  );
  const actionLabel = (key: string, action: string) =>
    action === 'uninstall' ? t('common.uninstall') : store.status[key] === 'needsUpdate' ? t('common.update') : t('common.install');
  const kindLabel = { install: 'run.installing', update: 'run.updating', uninstall: 'run.uninstalling', reinstall: 'run.reinstalling' } as const;
</script>

<section>
  <h1>{t('activity.title')}</h1>

  <h2>{t('activity.now')}</h2>
  {#if running}
    <div class="row now">
      <button class="open" onclick={() => onopen(running.key)} aria-label={t('common.open', { name: running.id })}>
        <AppIcon app={running} size={44} />
        <span class="text">
          <span class="name">{running.id}</span>
          <span class="meta">
            {run ? t(kindLabel[run.kind]) : t('run.starting')}
            {#if run?.steps.length} · {t('run.stepOf', { n: Math.min(done + 1, run.steps.length), total: run.steps.length })}{/if}
          </span>
        </span>
      </button>
      <ActionButton app={running} />
    </div>
  {:else}
    <p class="empty">{t('activity.nothingRunning')}</p>
  {/if}

  <div class="heading">
    <h2>{t('activity.upNext')} {#if queued.length}<span class="count">{queued.length}</span>{/if}</h2>
    {#if queued.length > 1}
      <button class="secondary" onclick={clearQueue}>{t('activity.clearQueue')}</button>
    {/if}
  </div>
  {#if queued.length}
    <ol class="queue">
      {#each queued as q, i (q.key)}
        <li class="row">
          <span class="position">{i + 1}</span>
          <button class="open" onclick={() => onopen(q.key)} aria-label={t('common.open', { name: q.app!.id })}>
            <AppIcon app={q.app!} size={36} />
            <span class="text">
              <span class="name">{q.app!.id}</span>
              <span class="meta">{actionLabel(q.key, q.action)} · {q.app!.collection}</span>
            </span>
          </button>
          <button class="secondary" aria-label={t('action.removeFromQueue', { app: q.app!.id })} onclick={() => dequeue(q.key)}>{t('common.remove')}</button>
        </li>
      {/each}
    </ol>
  {:else}
    <p class="empty">{t('activity.queueEmpty')}</p>
  {/if}
</section>

<style>
  section {
    max-width: 48rem;
  }
  h1 {
    margin: 0 0 1.5rem;
    font: 700 2rem/1.1 var(--font-display);
    letter-spacing: -0.02em;
  }
  h2 {
    margin: 1.75rem 0 0.75rem;
    font: 650 1rem var(--font-display);
  }
  .heading {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin: 1.75rem 0 0.75rem;
  }
  .heading h2 {
    margin: 0;
  }
  .count {
    margin-left: 0.4rem;
    padding: 0.05rem 0.5rem;
    border-radius: 999px;
    background: var(--hover);
    font-size: 0.8rem;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.75rem 1rem;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .now {
    box-shadow: var(--shadow-raised);
  }
  .queue {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.5rem;
  }
  .position {
    flex: none;
    width: 1.5rem;
    text-align: center;
    font: 700 0.9rem var(--font-display);
    color: var(--muted);
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.85rem;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }
  .text {
    display: grid;
    min-width: 0;
  }
  .name {
    font: 650 1.02rem var(--font-display);
  }
  .meta {
    font-size: 0.82rem;
    color: var(--muted);
  }
  .empty {
    color: var(--muted);
  }
</style>

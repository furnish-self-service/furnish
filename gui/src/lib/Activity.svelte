<script lang="ts">
  // Current, queued and recently completed operations.
  import { appStatus, store, dequeue, clearQueue, queueBlocker, recheckClosed, retryPrerequisite, isPending, retryActivity, appByKey, appName } from './state.svelte';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';
  import Plan from './Plan.svelte';
  import ActivityGroupRow from './ActivityGroupRow.svelte';
  import { isGroup, stepProgress } from './activity-history';
  import ActivityAttempt from './ActivityAttempt.svelte';
  import RunLog from './RunLog.svelte';
  import { t, lang, formatDate } from './i18n.svelte';

  let { onopen }: { onopen: (key: string) => void } = $props();

  let showSteps = $state(false);
  const runningKey = $derived(Object.keys(store.busy)[0]);
  const running = $derived(store.apps.find((a) => a.key === runningKey));
  const run = $derived(runningKey ? store.runs[runningKey] : undefined);
  const progress = $derived(stepProgress(run?.steps ?? []));
  const queued = $derived(
    store.queue.map((q) => ({ ...q, app: store.apps.find((a) => a.key === q.key) })).filter((q) => q.app),
  );
  const actionLabel = (key: string, action: string) =>
    action === 'uninstall' ? t('common.uninstall') : appStatus(key) === 'needsUpdate' ? t('common.update') : t('common.install');
  const kindLabel = { install: 'run.installing', update: 'run.updating', uninstall: 'run.uninstalling', reinstall: 'run.reinstalling' } as const;
</script>

<section>
  <h1>{t('activity.title')}</h1>

  {#if run}
    <h2>{t('activity.now')}</h2>
    <div class="row now">
      {#if running}
        <button class="open" onclick={() => onopen(running.key)} aria-label={t('common.open', { name: appName(running) })}>
          <AppIcon app={running} size={44} />
          <span class="text">
            <span class="name">{appName(running)}</span>
            <span class="meta">
              {t(kindLabel[run.kind])}
              {#if progress.total} · {t('run.stepOf', { n: progress.n, total: progress.total })}{/if}
            </span>
          </span>
        </button>
        <ActionButton app={running} />
      {:else}
        <span class="text"><span class="name">{run.name}</span><span class="meta">{t(kindLabel[run.kind])}</span></span>
      {/if}
    </div>
    <details class="steps" bind:open={showSteps}>
      <summary>{t('app.steps')}</summary>
      {#if showSteps}<Plan operation={run} />{/if}
    </details>
    <RunLog lines={store.logs[runningKey!] ?? []} busy />
  {:else if !queued.length}
    <p class="empty">{t('activity.idle')}</p>
  {/if}

  {#if queued.length}
  <div class="heading">
    <h2>{t('activity.upNext')} {#if queued.length}<span class="count">{queued.length}</span>{/if}</h2>
    {#if queued.length > 1}
      <button class="secondary" onclick={clearQueue}>{t('activity.clearQueue')}</button>
    {/if}
  </div>
    <ol class="queue">
      {#each queued as q, i (q.key)}
        {@const blocker = queueBlocker(q)}
        {@const dependency = blocker ? appByKey(blocker) : undefined}
        <li class="row">
          <span class="position">{i + 1}</span>
          <button class="open" onclick={() => onopen(q.key)} aria-label={t('common.open', { name: appName(q.app!) })}>
            <AppIcon app={q.app!} size={36} />
            <span class="text">
              <span class="name">{appName(q.app!)}</span>
              <span class="meta">{actionLabel(q.key, q.action)} · {q.app!.collection}</span>
              {#if q.waitingForClose?.length}
                <span class="meta">{t('queue.closeApps', {names: q.waitingForClose.join(', ')})}</span>
              {/if}
              {#if blocker}
                <span class="meta">{dependency ? t('queue.waitingFor', { app: appName(dependency) }) : t('queue.missing', { detail: blocker })}</span>
              {/if}
            </span>
          </button>
          {#if q.waitingForClose?.length}
            <button class="secondary" onclick={() => recheckClosed(q.key)}>{t('queue.recheckClosed')}</button>
          {:else if dependency && !isPending(dependency.key)}
            <button class="secondary" onclick={() => retryPrerequisite(dependency.key)}>{t('queue.retry', { app: appName(dependency) })}</button>
          {/if}
          <button class="secondary" aria-label={t('action.removeFromQueue', { app: appName(q.app!) })} onclick={() => dequeue(q.key)}>{t('common.remove')}</button>
        </li>
      {/each}
    </ol>

  {/if}

  {#if store.recent.length}
    <h2>{t('activity.recent')}</h2>
    <ol class="queue">
      {#each store.recent as entry (isGroup(entry) ? entry.id : entry.summary.id)}
        {#if isGroup(entry)}
          <ActivityGroupRow group={entry} />
        {:else}
          {@const item = entry.summary}
          <li class="history">
            <div class="name" class:failed={item.result === 'failed'}>
              {item.result === 'fallback'
                ? t('toast.fellBack', { app: item.name, date: formatDate(item.fallbackDate ?? '') })
                : t(`run.${item.result}.${item.kind}`, { app: item.name })}
            </div>
            <div class="meta">
              <time datetime={new Date(item.finishedAt).toISOString()}>
                {new Date(item.finishedAt).toLocaleString(lang(), { dateStyle: 'medium', timeStyle: 'short' })}
              </time>
              · {t(`activity.trigger.${item.trigger}`)} · {item.key}
            </div>
            <ActivityAttempt {entry} />
            {#if item.result === 'failed'}
              {#if appByKey(item.key) && appStatus(item.key) !== 'unavailable'}
                <button class="secondary" disabled={isPending(item.key) || store.retrying[item.key]}
                  onclick={() => retryActivity(item)}>{t('queue.retry', { app: item.name })}</button>
              {:else}
                <span class="meta">{t('activity.kitUnavailable')}</span>
              {/if}
            {/if}
          </li>
        {/if}
      {/each}
    </ol>
  {/if}
</section>

<style>
  section {
    max-width: 48rem;
  }
  h1 {
    margin: 0 0 1.5rem;
    font: var(--page-heading);
    letter-spacing: -0.03em;
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
  .steps, .history {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--surface);
    padding: 0.75rem 1rem;
  }
  .steps { margin-top: 0.5rem; }
  .steps summary { cursor: pointer; font-weight: 650; }
  .history .meta { margin: 0.25rem 0; overflow-wrap: anywhere; }
  .failed { color: var(--danger); }
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

<script lang="ts">
  import Plan from './Plan.svelte';
  import ActivityAttempt from './ActivityAttempt.svelte';
  import RunLog from './RunLog.svelte';
  import { groupProgress, itemDone, type ActivityGroup, type GroupItem } from './activity-history';
  import { appByKey, appName, isPending, kitName, queueBlocker, retryGroupItem, retryFailed, store } from './state.svelte';
  import { formatDate, lang, t } from './i18n.svelte';

  let { group }: { group: ActivityGroup } = $props();
  const kit = $derived(group.kind === 'kit' ? store.kits.find(k => group.kind === 'kit' && k.key === group.key) : undefined);
  const progress = $derived(groupProgress(group, store.queue, isPending));
  const failed = $derived(group.items.filter(i => i.state === 'failed'));
  const blockerNames = $derived([...new Set([...progress.blocked.values()].flat())].map(key => {
    const app = appByKey(key);
    return app ? appName(app) : group.items.find(item => item.key === key)?.name ?? key;
  }).join(', '));

  function name(item: GroupItem) {
    const app = appByKey(item.key);
    return app ? appName(app) : item.name;
  }

  function stateLabel(item: GroupItem) {
    const blocked = progress.blocked.get(item.key)?.[0];
    if (blocked) {
      const app = appByKey(blocked);
      return t('queue.waitingFor', { app: app ? appName(app) : blocked });
    }
    if (item.state === 'queued') {
      const queued = store.queue.find(q => q.key === item.key);
      const dependency = queued && queueBlocker(queued);
      const app = dependency && appByKey(dependency);
      if (dependency) return t('queue.waitingFor', { app: app ? appName(app) : dependency });
      return t('common.queued');
    }
    if (item.state === 'running') return t(group.kind === 'updates' && item.selected ? 'common.updating' : 'common.installing');
    if (item.state === 'skipped') return t('plan.state.skipped');
    if (item.state === 'interrupted') return t('activity.kitInterrupted');
    if (item.state === 'fallback' && item.attempts[0]?.summary.fallbackDate)
      return t('toast.fellBack', { app: name(item), date: formatDate(item.attempts[0].summary.fallbackDate) });
    if (itemDone(item)) return group.kind === 'updates' && item.selected ? t('run.done.update', { app: name(item) }) : t('common.installed');
    return t(item.state === 'failed' ? 'plan.state.failed' : 'plan.state.cancelled');
  }

  const date = (time: number) => new Date(time).toLocaleString(lang(), { dateStyle: 'medium', timeStyle: 'short' });
</script>

<li class="kit-history" data-kind={group.kind}>
  <details>
    <summary>
      {#if group.kind === 'updates'}
        <strong>{t('common.updateAll')}</strong> · {t('activity.updateProgress', { done: progress.done, total: progress.total, waiting: progress.waiting })}
        {#if progress.blocked.size} · {t('activity.blockedCount', { n: progress.blocked.size, apps: blockerNames })}{/if}
      {:else}
        <strong>{kit ? kitName(kit) : group.name}</strong> · {t('activity.kitProgress', { done: progress.done, total: progress.total })}
      {/if}
      {#each failed as item (item.key)}
        <span class="failed"> · {t(group.kind === 'updates' && item.selected ? 'run.failed.update' : 'run.failed.install', { app: name(item) })}</span>
      {/each}
    </summary>
    <div class="meta"><time datetime={new Date(group.updatedAt).toISOString()}>{date(group.updatedAt)}</time> · {t('activity.trigger.user')}{#if group.kind === 'kit'} · {group.key}{/if}</div>
    {#if group.kind === 'updates' && failed.length}
      <button class="secondary" disabled={failed.every(i => isPending(i.key) || store.retrying[i.key] || !appByKey(i.key) || store.status[i.key] === 'unavailable')} onclick={() => retryFailed(group)}>{t('activity.retryFailed')}</button>
    {/if}
    <ul>
      {#each group.items as item (item.key)}
        {@const run = store.runs[item.key]}
        <li data-app={item.key}>
          <strong>{name(item)}</strong> · {stateLabel(item)}
          {#if !item.selected}<small>{t('activity.kitPrerequisite')}</small>{/if}
          {#if item.state === 'failed' || item.state === 'cancelled' || item.state === 'interrupted'}
            {#if appByKey(item.key) && store.status[item.key] !== 'unavailable'}
              <button class="secondary" disabled={isPending(item.key) || store.retrying[item.key]} onclick={() => retryGroupItem(group, item.key)}>
                {t('queue.retry', { app: name(item) })}
              </button>
            {:else}
              <small>{t('activity.kitUnavailable')}</small>
            {/if}
          {/if}
          {#if item.state === 'running' && run && run.id === item.runId}
            <details>
              <summary>{t('app.steps')}</summary>
              <Plan operation={run} /><RunLog lines={store.logs[item.key] ?? []} busy />
            </details>
          {/if}
          {#each item.attempts as attempt (attempt.summary.id)}
            <ActivityAttempt entry={attempt} multiple={item.attempts.length > 1} />
          {/each}
        </li>
      {/each}
    </ul>
  </details>
</li>

<style>
  .kit-history { border: 1px solid var(--line); border-radius: var(--radius); background: var(--surface); padding: .75rem 1rem; }
  summary { cursor: pointer; }
  .meta, small { color: var(--muted); font-size: .82rem; }
  .meta { margin: .25rem 0; overflow-wrap: anywhere; }
  .failed { color: var(--danger); }
  ul { list-style: none; padding: 0; margin: .6rem 0 0; }
  ul > li { padding: .5rem 0; border-top: 1px solid var(--line); }
  button, small { margin-inline-start: .4rem; }
  li details { margin-block: .3rem; }
</style>

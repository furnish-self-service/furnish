<script lang="ts">
  import Plan from './Plan.svelte';
  import RunLog from './RunLog.svelte';
  import { kitItemDone, type KitActivity, type KitItem } from './activity-history';
  import { appByKey, appName, isPending, kitName, queueBlocker, retryKitItem, store } from './state.svelte';
  import { formatDate, lang, t } from './i18n.svelte';

  let { group }: { group: KitActivity } = $props();
  const kit = $derived(store.kits.find(k => k.key === group.key));
  const selected = $derived(group.items.filter(i => i.selected));
  const installed = $derived(selected.filter(kitItemDone).length);
  const failed = $derived(group.items.filter(i => i.state === 'failed'));
  let openAttempt = $state<string | null>(null);

  function name(item: KitItem) {
    const app = appByKey(item.key);
    return app ? appName(app) : item.name;
  }

  function stateLabel(item: KitItem) {
    if (item.state === 'queued') {
      const queued = store.queue.find(q => q.key === item.key);
      const dependency = queued && queueBlocker(queued);
      const app = dependency && appByKey(dependency);
      if (dependency) return t('queue.waitingFor', { app: app ? appName(app) : dependency });
      return t('common.queued');
    }
    if (item.state === 'running') return t('common.installing');
    if (item.state === 'interrupted') return t('activity.kitInterrupted');
    if (item.state === 'fallback' && item.attempts[0]?.summary.fallbackDate)
      return t('toast.fellBack', { app: name(item), date: formatDate(item.attempts[0].summary.fallbackDate) });
    if (kitItemDone(item)) return t('common.installed');
    return t(item.state === 'failed' ? 'plan.state.failed' : 'plan.state.cancelled');
  }

  const date = (time: number) => new Date(time).toLocaleString(lang(), { dateStyle: 'medium', timeStyle: 'short' });
</script>

<li class="kit-history">
  <details>
    <summary>
      <strong>{kit ? kitName(kit) : group.name}</strong> · {t('activity.kitProgress', { done: installed, total: selected.length })}
      {#each failed as item (item.key)}
        <span class="failed"> · {t('run.failed.install', { app: name(item) })}</span>
      {/each}
    </summary>
    <div class="meta"><time datetime={new Date(group.updatedAt).toISOString()}>{date(group.updatedAt)}</time> · {t('activity.trigger.user')} · {group.key}</div>
    <ul>
      {#each group.items as item (item.key)}
        {@const run = store.runs[item.key]}
        <li data-app={item.key}>
          <strong>{name(item)}</strong> · {stateLabel(item)}
          {#if !item.selected}<small>{t('activity.kitPrerequisite')}</small>{/if}
          {#if item.state === 'failed' || item.state === 'cancelled' || item.state === 'interrupted'}
            {#if appByKey(item.key) && store.status[item.key] !== 'unavailable'}
              <button class="secondary" disabled={isPending(item.key)} onclick={() => retryKitItem(group, item.key)}>
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
            {#if attempt.summary.failedStep}<div class="failed">{t('activity.failedStep', { step: attempt.summary.failedStep })}</div>{/if}
            {#if attempt.details}
              <button class="secondary" aria-expanded={openAttempt === attempt.summary.id}
                onclick={() => openAttempt = openAttempt === attempt.summary.id ? null : attempt.summary.id}>
                {t('app.steps')}{#if item.attempts.length > 1} · {date(attempt.summary.finishedAt)} · {t(`plan.state.${attempt.summary.result === 'fallback' ? 'done' : attempt.summary.result}`)}{/if}
              </button>
              {#if openAttempt === attempt.summary.id}
                <Plan operation={attempt.details.run} /><RunLog lines={attempt.details.logs} />
              {/if}
            {/if}
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

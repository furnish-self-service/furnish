<script lang="ts">
  import type { RecentActivity } from './activity-history';
  import { lang, t } from './i18n.svelte';
  import Plan from './Plan.svelte';
  import RunLog from './RunLog.svelte';

  let { entry, multiple = false }: { entry: RecentActivity; multiple?: boolean } = $props();
  const summary = $derived(entry.summary);
  let expanded = $state(false);
</script>

{#if summary.collectionRevision}
  <div class="meta">{t('collections.revision')} <code title={summary.collectionRevision}>{summary.collectionRevision.slice(0, 12)}</code></div>
{/if}
{#if summary.requiresNewSession && (!expanded || !entry.details)}<p>{t('session.after')}</p>{/if}
{#if summary.failedStep}<div class="failed">{t('activity.failedStep', { step: summary.failedStep })}</div>{/if}
{#if summary.failure}<div class="failed">{t(`activity.failure.${summary.failure}`)}</div>{/if}
{#if entry.details || summary.steps?.length}
  <button class="secondary reveal" aria-expanded={expanded} onclick={() => expanded = !expanded}>
    {t('app.steps')}{#if multiple} · {new Date(summary.finishedAt).toLocaleString(lang())} · {t(`plan.state.${summary.result === 'fallback' ? 'done' : summary.result}`)}{/if}
  </button>
  {#if expanded}
    {#if entry.details}
      <Plan operation={entry.details.run} /><RunLog lines={entry.details.logs} />
    {:else}
      <ol class="saved-steps">
        {#each summary.steps ?? [] as step}
          <li>{step.name} · {t(step.state === 'notRun' ? 'plan.notRun' : `plan.state.${step.state}`)}</li>
        {/each}
      </ol>
    {/if}
  {/if}
{/if}

<style>
  .meta { color: var(--muted); font-size: .82rem; margin: .25rem 0; }
  .failed { color: var(--danger); }
  .reveal { margin-block: .4rem; }
  .saved-steps { padding-inline-start: 1.6rem; }
</style>

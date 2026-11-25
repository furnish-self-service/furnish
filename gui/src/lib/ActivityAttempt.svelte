<script lang="ts">
  import { type RecentActivity, stepProgress } from './activity-history';
  import { formatDuration, lang, t } from './i18n.svelte';
  import { packageUpdateLabel } from './state.svelte';
  import Plan from './Plan.svelte';
  import PlanStep from './PlanStep.svelte';
  import RunLog from './RunLog.svelte';

  let { entry, multiple = false }: { entry: RecentActivity; multiple?: boolean } = $props();
  const summary = $derived(entry.summary);
  const numbers = $derived(stepProgress((summary.steps ?? []).map(step => ({ step, state: step.state }))).numbers);
  let expanded = $state(false);
</script>

{#if packageUpdateLabel(summary.key)}
  <div class="meta">{t('activity.currentState', { state: packageUpdateLabel(summary.key) ?? '' })}</div>
{/if}
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
      <div class="saved-steps">
        <ol class="plan-steps">
          {#each summary.steps ?? [] as step, i}
            <PlanStep name={step.name} type={step.type} action={step.action} state={step.state} number={numbers[i]}>
              {#snippet timing()}
                <strong>{step.state === 'done' && step.outcome ? t(`plan.result.${step.outcome}`)
                  : step.state === 'notRun' ? t('plan.notRun') : t(`plan.state.${step.state}`)}</strong>
                {#if step.durationMs !== undefined}<span>{formatDuration(step.durationMs)}</span>{/if}
              {/snippet}
            </PlanStep>
          {/each}
        </ol>
      </div>
    {/if}
  {/if}
{/if}

<style>
  .meta { color: var(--muted); font-size: .82rem; margin: .25rem 0; }
  .failed { color: var(--danger); }
  .reveal { margin-block: .4rem; }
  .saved-steps { container-type: inline-size; margin-block-start: .75rem; }
</style>

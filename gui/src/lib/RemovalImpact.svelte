<script lang="ts">
  import { appName, kitName, removalImpact } from './state.svelte';
  import { plan } from './api';
  import { lang, t, terror } from './i18n.svelte';

  let { keys, removingKit, ready = $bindable(false) }: { keys: string[]; removingKit?: string; ready?: boolean } = $props();
  const impact = $derived(removalImpact(keys, removingKit));
  const names = (values: string[]) => new Intl.ListFormat(lang()).format(values);
  const selection = $derived(JSON.stringify(keys));
  let targets = $state<{ key: string; code: 'package.removal' | 'package.unrecordedRemoval'; manager: string; name: string }[]>([]);
  let failure = $state('');
  let attempt = $state(0);
  $effect(() => {
    const apps: string[] = JSON.parse(selection);
    attempt;
    let current = true;
    ready = false;
    failure = '';
    targets = [];
    Promise.all(apps.map(async key => {
      const steps = await plan(key, true);
      return steps.flatMap(step => (step.selectionNotes ?? []).flatMap(({ code, params }) =>
        (code === 'package.removal' || code === 'package.unrecordedRemoval')
        ? [{ key, code: code as 'package.removal' | 'package.unrecordedRemoval', manager: String(params.manager), name: String(params.name) }] : []));
    })).then(results => {
      if (current) { targets = results.flat(); ready = true; }
    }).catch(error => { if (current) failure = String(error); });
    return () => { current = false; ready = false; };
  });
</script>

{#if failure}
  <p role="alert">{t('plan.previewError', { error: terror(failure) })}</p>
  <button class="secondary" onclick={() => attempt++}>{t('status.retry')}</button>
{:else if !ready}
  <p role="status">{t('plan.preparing')}</p>
{:else if targets.length}
  <ul>
    {#each targets as target}
      <li>{#if keys.length > 1}<strong>{target.key}</strong> · {/if}{t(`plan.note.${target.code}`, { manager: target.manager, name: target.name })}</li>
    {/each}
  </ul>
{/if}

{#if impact.apps.length || impact.kits.length}
  <div class="impact" role="status">
    {#if impact.apps.length}<p>{t('uninstall.requiredBy', { apps: names(impact.apps.map(appName)) })}</p>{/if}
    {#if impact.kits.length}<p>{t('uninstall.incompleteKits', { kits: names(impact.kits.map(kitName)) })}</p>{/if}
  </div>
{/if}

<style>
  .impact {
    padding-inline-start: 0.75rem;
    border-inline-start: 0.2rem solid var(--highlight-ink);
    text-align: start;
  }
  p { margin: 0.3rem 0; }
</style>

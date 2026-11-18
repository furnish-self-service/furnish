<script lang="ts">
  import { appName, kitName, removalImpact } from './state.svelte';
  import { lang, t } from './i18n.svelte';

  let { keys, removingKit }: { keys: string[]; removingKit?: string } = $props();
  const impact = $derived(removalImpact(keys, removingKit));
  const names = (values: string[]) => new Intl.ListFormat(lang()).format(values);
</script>

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

<script lang="ts">
  import { actionUrl, linkActions, type ActionLink, type LinkAction } from './action-link';
  import { t } from './i18n.svelte';
  import { toast } from './state.svelte';

  let { kind, collection, id }: Omit<ActionLink, 'action'> = $props();
  let action = $state<LinkAction>('install');
  const url = $derived(actionUrl({ kind, collection, id, action }));
  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
      toast('success', t('common.copied'));
    } catch (error) {
      toast('error', t('common.copy'), String(error));
    }
  }
</script>

<details class="action-links">
  <summary>{t('link.share')}</summary>
  <div>
    <select aria-label={t('link.share')} bind:value={action}>
      {#each linkActions as verb}<option value={verb}>{t(`common.${verb}`)}</option>{/each}
    </select>
    <input aria-label={t('link.share')} readonly value={url} onfocus={(e) => e.currentTarget.select()} />
    <button class="secondary" onclick={copy}>{t('common.copy')}</button>
  </div>
</details>

<style>
  .action-links { margin-block: 1.25rem; }
  summary { width: fit-content; cursor: pointer; }
  div { display: flex; flex-wrap: wrap; gap: .5rem; margin-top: .7rem; }
  input { min-width: 8rem; flex: 1; font-family: var(--font-mono); }
</style>

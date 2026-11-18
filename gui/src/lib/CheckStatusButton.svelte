<script lang="ts">
  import { checkStatus, isPending, store } from './state.svelte';
  import { t } from './i18n.svelte';

  let { appKey }: { appKey: string } = $props();
  const checking = $derived(!!store.checkingStatus[appKey]);
</script>

<button class="secondary" disabled={checking || isPending(appKey)} aria-busy={checking}
  onclick={() => checkStatus(appKey)}>
  {checking ? t('catalog.checking') : t(store.statusErrors[appKey] === undefined ? 'status.check' : 'status.retry')}
</button>

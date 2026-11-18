<script lang="ts">
  import type { AppInfo, Option } from './api';
  import { t } from './i18n.svelte';
  import { optionLabel, optionValueLabel, store } from './state.svelte';

  let { app }: { app: AppInfo } = $props();
  const changed = $derived(app.options.filter(o => Object.hasOwn(app.optionChanges, o.name)));
  const collectionOptions = $derived(store.collections.find(c => c.name === app.collection)?.options);

  function source(option: Option) {
    switch (option.source) {
      case 'collection': return t('options.fromCollection', { collection: app.collection });
      case 'app': return t('options.fromApp');
      case 'policy': return t('common.setByOrg');
      case 'default':
        return collectionOptions?.some(o => o.name === option.name && o.default != null)
          ? t('options.fromCollection', { collection: app.collection })
          : t('options.default');
    }
  }
</script>

<table class="changes">
  <thead>
    <tr><th scope="col">{t('options.option')}</th><th scope="col">{t('common.installed')}</th><th scope="col">{t('options.afterReinstall')}</th></tr>
  </thead>
  <tbody>
    {#each changed as option (option.name)}
      <tr>
        <th scope="row">{optionLabel(app.collection, option)}</th>
        <td>{app.optionChanges[option.name] == null ? t('options.notRecorded') : optionValueLabel(app.collection, option, app.optionChanges[option.name])}</td>
        <td>{optionValueLabel(app.collection, option, option.value)}
          {#if option.value != null}<small>{source(option)}</small>{/if}
        </td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
  .changes {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    color: var(--ink);
    font-size: 0.86rem;
    text-align: start;
    overflow-wrap: anywhere;
  }
  th, td {
    padding: 0.45rem;
    border-top: 1px solid var(--line);
    text-align: start;
    vertical-align: top;
  }
  td { font-weight: 400; }
  small { display: block; color: var(--ink-soft); }
</style>

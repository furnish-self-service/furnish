<script lang="ts">
  // How much of a collection furnish downloads, and what it does when installing one of its apps
  // fails. Values the organization sets per collection or in general are shown, not editable.
  import * as api from './api';
  import { store, toast } from './state.svelte';
  import { t } from './i18n.svelte';

  let { collection }: { collection: api.CollectionInfo } = $props();

  type Key = keyof api.CollectionSettings;
  // A function, so the texts follow the language.
  const rows = (): { key: Key; label: string; desc: string; choices: [unknown, string][] }[] => [
    {
      key: 'fallback',
      label: t('cs.fallback'),
      desc: t('cs.fallbackDesc'),
      choices: [
        ['ask', t('cs.fallbackAsk')],
        ['automatic', t('cs.fallbackAutomatic')],
        ['off', t('cs.fallbackOff')],
      ],
    },
    {
      key: 'fallbackMaxAgeDays',
      label: t('cs.maxAge'),
      desc: t('cs.maxAgeDesc'),
      choices: [
        [0, t('cs.anyAge')],
        [7, t('cs.upToDays', { n: 7 })],
        [30, t('cs.upToDays', { n: 30 })],
        [90, t('cs.upToDays', { n: 90 })],
      ],
    },
    {
      key: 'blobsOnDemand',
      label: t('cs.blobs'),
      desc: t('cs.blobsDesc'),
      choices: [
        [true, t('cs.blobsOnDemand')],
        [false, t('cs.blobsAll')],
      ],
    },
    {
      key: 'checkoutDepth',
      label: t('cs.history'),
      desc: t('cs.historyDesc'),
      choices: [
        [0, t('cs.allCommits')],
        [100, t('cs.lastCommits', { n: 100 })],
        [20, t('cs.lastCommits', { n: 20 })],
        [1, t('cs.latestOnly')],
      ],
    },
  ];
  type Row = ReturnType<typeof rows>[number];

  const value = (key: Key) => collection.settings.settings[key];
  // A value set outside the app (settings.json, the policy) still shows up as a choice.
  const choices = (row: Row) =>
    row.choices.some(([v]) => v === value(row.key)) ? row.choices : [...row.choices, [value(row.key), String(value(row.key))] as [unknown, string]];
  const label = (row: Row) => choices(row).find(([v]) => v === value(row.key))?.[1];
  let saving = $state(false);

  async function change(key: Key, raw: string) {
    saving = true;
    try {
      await api.setCollectionSetting(collection.name, key, JSON.parse(raw));
      store.collections = await api.listCollections();
    } catch (e) {
      toast('error', t('cs.changeFailed', { name: collection.name }), String(e));
    }
    saving = false;
  }
</script>

<ul class="settings">
  {#each rows() as row (row.key)}
    <li>
      <label for="cs-{collection.name}-{row.key}">
        <span class="label">{row.label}</span>
        <span class="desc">{row.desc}</span>
      </label>
      {#if collection.settings.locked.includes(row.key)}
        <span class="value">{label(row)} <span class="org">{t('common.setByOrg')}</span></span>
      {:else}
        <select
          id="cs-{collection.name}-{row.key}"
          disabled={saving}
          value={JSON.stringify(value(row.key))}
          onchange={(e) => change(row.key, e.currentTarget.value)}
        >
          {#each choices(row) as [v, text]}<option value={JSON.stringify(v)}>{text}</option>{/each}
        </select>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .settings {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem 1.5rem;
    padding: 0.75rem 0;
    border-bottom: 1px solid var(--line);
  }
  label {
    display: grid;
    gap: 0.15rem;
    flex: 1;
    min-width: 14rem;
  }
  .label,
  .value {
    font-weight: 600;
  }
  .desc {
    font-size: 0.85rem;
    color: var(--muted);
  }
  select {
    padding: 0.45rem 0.7rem;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: 0.92rem var(--font-body);
  }
  .org {
    margin-left: 0.4rem;
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 650;
    color: var(--ink-soft);
    background: var(--hover);
    white-space: nowrap;
  }
</style>

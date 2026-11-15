<script lang="ts">
  // How much of a collection furnish downloads, and what it does when installing one of its apps
  // fails. Values the organization sets per collection or in general are shown, not editable.
  import * as api from './api';
  import { store, toast } from './state.svelte';

  let { collection }: { collection: api.CollectionInfo } = $props();

  type Key = keyof api.CollectionSettings;
  const rows: { key: Key; label: string; desc: string; choices: [unknown, string][] }[] = [
    {
      key: 'fallback',
      label: 'When installing an app fails',
      desc: 'An earlier version of the app may still work.',
      choices: [
        ['ask', 'Offer the previous version'],
        ['automatic', 'Install the previous version'],
        ['off', 'Nothing'],
      ],
    },
    {
      key: 'fallbackMaxAgeDays',
      label: 'Earlier versions',
      desc: 'How much older than the failing version the previous one may be.',
      choices: [
        [0, 'Any age'],
        [7, 'Up to 7 days older'],
        [30, 'Up to 30 days older'],
        [90, 'Up to 90 days older'],
      ],
    },
    {
      key: 'blobsOnDemand',
      label: 'Installer files',
      desc: 'Installer files (.msi, .deb, …) and earlier versions take space on this computer.',
      choices: [
        [true, 'Download when an app needs them'],
        [false, 'Download all with the collection'],
      ],
    },
    {
      key: 'checkoutDepth',
      label: 'History',
      desc: 'Earlier versions come from the commits furnish keeps.',
      choices: [
        [0, 'All commits'],
        [100, 'Last 100 commits'],
        [20, 'Last 20 commits'],
        [1, 'Latest commit only'],
      ],
    },
  ];

  const value = (key: Key) => collection.settings.settings[key];
  // A value set outside the app (settings.json, the policy) still shows up as a choice.
  const choices = (row: (typeof rows)[number]) =>
    row.choices.some(([v]) => v === value(row.key)) ? row.choices : [...row.choices, [value(row.key), String(value(row.key))] as [unknown, string]];
  const label = (row: (typeof rows)[number]) => choices(row).find(([v]) => v === value(row.key))?.[1];
  let saving = $state(false);

  async function change(key: Key, raw: string) {
    saving = true;
    try {
      await api.setCollectionSetting(collection.name, key, JSON.parse(raw));
      store.collections = await api.listCollections();
    } catch (e) {
      toast('error', `Couldn't change the setting for ${collection.name}`, String(e));
    }
    saving = false;
  }
</script>

<ul class="settings">
  {#each rows as row (row.key)}
    <li>
      <label for="cs-{collection.name}-{row.key}">
        <span class="label">{row.label}</span>
        <span class="desc">{row.desc}</span>
      </label>
      {#if collection.settings.locked.includes(row.key)}
        <span class="value">{label(row)} <span class="org">Set by your organization</span></span>
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

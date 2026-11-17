<script lang="ts">
  // The versions of an app people can install, from the collection's history (src/catalog.rs,
  // App::versions). Mounted only while shown: listing may download earlier files and ask DSC.
  import * as api from './api';
  import type { AppInfo, Version } from './api';
  import { store, appName, installVersion, isPending } from './state.svelte';
  import { describeNeeds } from './required';
  import { t, terror, formatDate, lang } from './i18n.svelte';

  let { app }: { app: AppInfo } = $props();

  let versions = $state<Version[] | null>(null);
  let error = $state<string | null>(null);

  // Load when shown, and again after an install changed which version the app is at.
  $effect(() => {
    void app.held?.commit;
    void store.status[app.key];
    error = null;
    api
      .appVersions(app.key)
      .then((v) => (versions = v))
      .catch((e) => ((error = String(e)), (versions = null)));
  });

  // The version installed: the one the app is held at, else the current one if it is up to date.
  const installedVersion = (v: Version) =>
    app.held ? app.held.commit === v.commit : store.status[app.key] === 'installed' && !!v.current;
</script>

{#if error}
  <p class="empty">{terror(error)}</p>
{:else if !versions}
  <p class="empty">{t('versions.loading')}</p>
{:else}
  <p class="lead">{t('versions.lead', { app: appName(app) })}</p>
  <ul class="versions">
    {#each versions as v (v.commit)}
      <li>
        <span class="when">
          <span class="date">{formatDate(v.date)}</span>
          {#if v.current}<span class="badge">{t('versions.current')}</span>{/if}
          {#if installedVersion(v)}<span class="badge installed">{t('versions.installed')}</span>{/if}
          {#if v.stated?.length}<span class="stated">{t('versions.stated', { versions: describeNeeds(lang(), v.stated) })}</span>{/if}
        </span>
        <span class="commit">{v.commit.slice(0, 7)}</span>
        {#if !installedVersion(v)}
          <button class="secondary" disabled={isPending(app.key)} onclick={() => installVersion(app, v.commit)}>
            {isPending(app.key) ? t('common.installing') : t('versions.install')}
          </button>
        {/if}
      </li>
    {/each}
  </ul>
  {#if versions.length === 1}<p class="empty">{t('versions.none', { app: appName(app) })}</p>{/if}
{/if}

<style>
  .lead {
    margin: 0.75rem 0 0.6rem;
    font-size: 0.85rem;
    color: var(--muted);
  }
  .versions {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem 1rem;
    min-height: 2.4rem;
    padding: 0.5rem 0;
    border-bottom: 1px solid var(--line);
  }
  .when {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.3rem 0.6rem;
    flex: 1;
    min-width: 12rem;
  }
  .date {
    font-weight: 600;
  }
  .stated {
    font-size: 0.85rem;
    color: var(--ink-soft);
  }
  .badge {
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 650;
    color: var(--ink-soft);
    background: var(--hover);
    white-space: nowrap;
  }
  .badge.installed {
    color: var(--accent-ink);
  }
  .commit {
    font: 0.78rem var(--font-mono);
    color: var(--muted);
  }
  .empty {
    color: var(--muted);
    font-size: 0.9rem;
  }
</style>

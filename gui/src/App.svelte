<script lang="ts">
  import logo from '../../logo.svg';
  import { store, start, visibleApps, type View } from './lib/state.svelte';
  import AppCard from './lib/AppCard.svelte';
  import AppDetail from './lib/AppDetail.svelte';
  import Collections from './lib/Collections.svelte';
  import Activity from './lib/Activity.svelte';
  import Toasts from './lib/Toasts.svelte';

  start();

  const view = $derived(store.view);
  const categories = $derived(
    [...new Set(store.apps.filter((a) => store.status[a.key] !== 'unavailable').flatMap((a) => a.categories))].sort(),
  );
  const count = (page: 'installed' | 'updates') => visibleApps({ page }, '').length;
  const activity = $derived(Object.keys(store.busy).length + store.queue.length);
  const apps = $derived(visibleApps(view, store.query));
  const detail = $derived(view.page === 'app' ? store.apps.find((a) => a.key === view.key) : undefined);

  const titles: Record<string, string> = { discover: 'Discover', installed: 'Installed', updates: 'Updates' };
  const title = $derived(view.page === 'category' ? view.name : titles[view.page] ?? '');

  function go(next: View) {
    store.view = next;
  }
  const isActive = (page: string, name?: string) =>
    view.page === page && (name === undefined || (view.page === 'category' && view.name === name));
</script>

<div class="shell">
  <nav aria-label="Sections">
    <button class="brand" onclick={() => go({ page: 'discover' })} aria-label="furnish, go to Discover">
      <img src={logo} alt="" width="34" height="34" />
      <span>furnish</span>
    </button>

    <button class:active={isActive('discover')} onclick={() => go({ page: 'discover' })}>Discover</button>
    <button class:active={isActive('installed')} onclick={() => go({ page: 'installed' })}>
      Installed <span class="count">{count('installed') || ''}</span>
    </button>
    <button class:active={isActive('updates')} onclick={() => go({ page: 'updates' })}>
      Updates
      {#if count('updates')}<span class="count pending">{count('updates')}</span>{/if}
    </button>

    <button class:active={isActive('activity')} onclick={() => go({ page: 'activity' })}>
      Activity
      {#if activity}<span class="count running">{activity}</span>{/if}
    </button>

    {#if categories.length}
      <h2>Categories</h2>
      {#each categories as name}
        <button class:active={isActive('category', name)} onclick={() => go({ page: 'category', name })}>{name}</button>
      {/each}
    {/if}

    <div class="spacer"></div>
    <button class:active={isActive('collections')} onclick={() => go({ page: 'collections' })}>Collections</button>
  </nav>

  <main>
    {#if view.page === 'collections'}
      <Collections />
    {:else if view.page === 'activity'}
      <Activity onopen={(key) => go({ page: 'app', key, back: view })} />
    {:else if view.page === 'app' && detail}
      <AppDetail app={detail} onback={() => view.page === 'app' && go(view.back)} />
    {:else}
      <div class="top">
        <h1>{title}</h1>
        <input
          class="search"
          type="search"
          placeholder="Search apps"
          aria-label="Search apps"
          bind:value={store.query}
        />
      </div>

      {#if store.loading}
        <p class="empty">Loading the catalog…</p>
      {:else if !store.collections.length}
        <div class="empty">
          <h2>Add a collection to see apps</h2>
          <p>Your organization publishes its apps in a git repository. Add its URL to get started.</p>
          <button class="primary" onclick={() => go({ page: 'collections' })}>Add a collection</button>
        </div>
      {:else if !apps.length}
        <p class="empty">
          {store.query ? `No apps match “${store.query}”.` : view.page === 'updates' ? 'All apps are up to date.' : 'No apps here yet.'}
        </p>
      {:else}
        <div class="grid">
          {#each apps as app (app.key)}
            <AppCard {app} onopen={() => go({ page: 'app', key: app.key, back: view })} />
          {/each}
        </div>
      {/if}
    {/if}
  </main>
</div>

<Toasts />

<style>
  .shell {
    display: grid;
    grid-template-columns: 14.5rem 1fr;
    height: 100vh;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 1.25rem 0.85rem;
    background: var(--sidebar);
    border-right: 1px solid var(--line);
    overflow-y: auto;
  }
  nav button.brand {
    justify-content: flex-start;
    margin-bottom: 1.25rem;
    padding: 0.25rem 0.5rem;
    color: var(--ink);
    font: 700 1.35rem var(--font-display);
  }
  nav button.brand:hover {
    background: none;
    color: var(--accent-ink);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font: 700 1.35rem var(--font-display);
    letter-spacing: -0.02em;
  }
  .brand img {
    border-radius: 9px;
  }
  nav h2 {
    margin: 1.4rem 0.6rem 0.35rem;
    font-size: 0.7rem;
    font-weight: 650;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--muted);
  }
  nav button {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.5rem 0.65rem;
    border: 0;
    border-radius: 9px;
    background: none;
    color: var(--ink-soft);
    font: 500 0.92rem var(--font-body);
    text-align: left;
    cursor: pointer;
  }
  nav button:hover {
    background: var(--hover);
  }
  nav button.active {
    background: var(--accent-soft);
    color: var(--accent-ink);
    font-weight: 650;
  }
  .count {
    font-size: 0.78rem;
    color: var(--muted);
  }
  .count.pending {
    padding: 0.05rem 0.45rem;
    border-radius: 999px;
    background: var(--highlight);
    color: var(--on-highlight);
    font-weight: 700;
  }
  .count.running {
    padding: 0.05rem 0.45rem;
    border-radius: 999px;
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 700;
  }
  .spacer {
    flex: 1;
  }
  main {
    padding: 2rem 2.25rem 4rem;
    overflow-y: auto;
  }
  .top {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.75rem;
  }
  h1 {
    margin: 0;
    font: 700 2rem/1.1 var(--font-display);
    letter-spacing: -0.02em;
  }
  .search {
    width: min(20rem, 100%);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(16.5rem, 1fr));
    gap: 1rem;
  }
  .empty {
    color: var(--muted);
  }
  div.empty {
    max-width: 28rem;
    margin-top: 3rem;
  }
  div.empty h2 {
    margin: 0;
    font: 650 1.3rem var(--font-display);
    color: var(--ink);
  }
  div.empty p {
    line-height: 1.5;
  }
  @media (max-width: 760px) {
    .shell {
      grid-template-columns: 11rem 1fr;
    }
    main {
      padding: 1.5rem 1.25rem 3rem;
    }
  }
</style>

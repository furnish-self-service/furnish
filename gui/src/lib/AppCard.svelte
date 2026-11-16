<script lang="ts">
  import type { AppInfo } from './api';
  import { store, appName, appDescription, categoryName } from './state.svelte';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';
  import CollectionAvatar from './CollectionAvatar.svelte';
  import { t } from './i18n.svelte';

  let { app, onopen }: { app: AppInfo; onopen: () => void } = $props();
  const status = $derived(store.status[app.key]);
</script>

<article class="card">
  <button class="open" onclick={onopen} aria-label={t('common.open', { name: appName(app) })}></button>
  <header>
    <AppIcon {app} />
    <div class="title">
      <h3>{appName(app)}</h3>
      {#if app.recommended}<span class="recommended">{t('card.recommended')}</span>{/if}
      <p class="meta">{app.categories.map((c) => categoryName(c, app.collection)).join(' · ') || app.collection}</p>
    </div>
  </header>
  <p class="description">{appDescription(app) || t('common.noDescription')}</p>
  <footer>
    {#if status === 'installed'}
      <span class="state installed">{store.declared[app.key] ? t(store.declared[app.key].by === 'nixos' ? 'app.status.nixos' : 'app.status.homeManager') : t('common.installed')}</span>
    {:else if status === 'needsUpdate'}
      <span class="state update">{t('common.updateAvailable')}</span>
    {:else}
      <span class="collection"><CollectionAvatar name={app.collection} /> {app.collection}</span>
    {/if}
    <!-- Uninstalling is only offered on the detail page, behind a confirmation. -->
    {#if status !== 'installed' || store.busy[app.key] || store.queue.some((q) => q.key === app.key)}<ActionButton {app} />{/if}
  </footer>
</article>

<style>
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1.1rem;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--line);
    box-shadow: var(--shadow);
    transition: transform 0.15s, box-shadow 0.15s;
  }
  .card:hover {
    transform: translateY(-2px);
    box-shadow: var(--shadow-raised);
  }
  @media (prefers-reduced-motion: reduce) {
    .card:hover { transform: none; }
  }
  /* Whole card opens the detail page; the action button sits above this layer. */
  .open {
    position: absolute;
    inset: 0;
    border: 0;
    border-radius: inherit;
    background: none;
    cursor: pointer;
  }
  header {
    display: flex;
    gap: 0.85rem;
    align-items: center;
  }
  .title {
    min-width: 0;
    flex: 1;
  }
  h3 {
    margin: 0;
    font: 650 1.12rem/1.2 var(--font-display);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    margin: 0.15rem 0 0;
    font-size: 0.8rem;
    color: var(--muted);
  }
  .description {
    margin: 0;
    flex: 1;
    font-size: 0.9rem;
    line-height: 1.45;
    color: var(--ink-soft);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
  }
  footer :global(button) {
    position: relative;
    z-index: 1;
  }
  .collection {
    font: 0.75rem var(--font-mono);
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  footer {
    min-height: 2rem;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.82rem;
    font-weight: 650;
  }
  .state::before {
    content: '';
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 50%;
    background: currentColor;
  }
  .installed {
    color: var(--accent-ink);
  }
  .update {
    color: var(--highlight-ink);
  }
  .recommended {
    font-size: 0.72rem;
    font-weight: 700;
    color: var(--accent-ink);
  }
</style>

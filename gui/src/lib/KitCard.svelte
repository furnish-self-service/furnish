<script lang="ts">
  import type { KitInfo } from './api';
  import { appByKey, kitStatus, kitName, kitDescription, categoryName } from './state.svelte';
  import AppIcon from './AppIcon.svelte';
  import { t } from './i18n.svelte';

  let { kit, onopen }: { kit: KitInfo; onopen: () => void } = $props();

  const apps = $derived(kit.items.map((i) => appByKey(i.key)).filter((a) => !!a));
  const status = $derived(kitStatus(kit));
</script>

<article class="card kit">
  <button class="open" onclick={onopen} aria-label={t('kit.openKit', { name: kitName(kit) })}></button>
  <header>
    <AppIcon app={kit} name={kitName(kit)} />
    <div class="title">
      <p class="eyebrow">{t('kit.eyebrow', { apps: t('common.apps', { n: kit.items.length }) })}</p>
      <h3>{kitName(kit)}</h3>
    </div>
  </header>
  <p class="description">{kitDescription(kit) || kit.categories.map((c) => categoryName(c, kit.collection)).join(' · ')}</p>
  <div class="stack" aria-hidden="true">
    {#each apps.slice(0, 5) as app (app.key)}<span class="mini"><AppIcon {app} size={26} /></span>{/each}
    {#if apps.length > 5}<span class="more">+{apps.length - 5}</span>{/if}
  </div>
  <footer>
    {#if status.installed}
      <span class="state" class:installed={status.complete} class:partial={!status.complete}>
        {t('kit.partlyInstalled', { n: status.installed, total: status.applicable })}
        {#if status.complete && status.installed < status.applicable}
          <small>{t('kit.optionalRemaining')}</small>
        {/if}
      </span>
    {:else}
      <span class="collection">{kit.collection}</span>
    {/if}
    <button class="choose" onclick={(e) => { e.stopPropagation(); onopen(); }}>{t('kit.chooseApps')}</button>
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
  /* Kits read as a set: a second sheet peeks out behind the card. */
  .kit {
    box-shadow: 5px 5px 0 -1px var(--surface), 5px 5px 0 0 var(--line), var(--shadow);
  }
  .card:hover {
    transform: translateY(-2px);
  }
  @media (prefers-reduced-motion: reduce) {
    .card:hover { transform: none; }
  }
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
  }
  .eyebrow {
    margin: 0;
    font: 700 0.68rem var(--font-body);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--accent-ink);
  }
  h3 {
    margin: 0.1rem 0 0;
    font: 650 1.12rem/1.2 var(--font-display);
  }
  .description {
    margin: 0;
    font-size: 0.9rem;
    line-height: 1.45;
    color: var(--ink-soft);
  }
  .stack {
    display: flex;
    align-items: center;
    padding-left: 0.3rem;
  }
  .mini {
    margin-left: -0.3rem;
    border-radius: 9px;
    box-shadow: 0 0 0 2px var(--surface);
    line-height: 0;
  }
  .more {
    margin-left: 0.4rem;
    font-size: 0.78rem;
    color: var(--muted);
  }
  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
    min-height: 2rem;
  }
  .collection {
    font: 0.75rem var(--font-mono);
    color: var(--muted);
  }
  .state {
    font-size: 0.82rem;
    font-weight: 650;
  }
  .state small { display: block; color: var(--muted); }
  .installed {
    color: var(--accent-ink);
  }
  .partial {
    color: var(--ink-soft);
  }
  .choose {
    position: relative;
    z-index: 1;
    flex: none;
    width: 8.25rem;
    height: 2.1rem;
    border: 1px solid var(--accent);
    border-radius: 999px;
    background: transparent;
    color: var(--accent-ink);
    font: 600 0.85rem var(--font-body);
    cursor: pointer;
  }
  .choose:hover {
    background: var(--accent-soft);
  }
</style>

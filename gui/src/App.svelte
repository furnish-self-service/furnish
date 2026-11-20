<script lang="ts">
  import { tick } from 'svelte';
  import logo from '../../logo.svg';
  import { appStatus, store, start, visibleApps, visibleKits, availableAppUpdates, updateAllApps, checkForUpdates, checkInstalledDesktop, restartInstalledDesktop, desktopIdle, trayTexts, syncTray, categoryName, collectionUpdates, type View } from './lib/state.svelte';
  import DepsDialog from './lib/DepsDialog.svelte';
  import QuestionsDialog from './lib/QuestionsDialog.svelte';
  import SignInBanner from './lib/SignInBanner.svelte';
  import LinkDialog from './lib/LinkDialog.svelte';
  import AppCard from './lib/AppCard.svelte';
  import AppDetail from './lib/AppDetail.svelte';
  import Collections from './lib/Collections.svelte';
  import Settings from './lib/Settings.svelte';
  import Activity from './lib/Activity.svelte';
  import KitCard from './lib/KitCard.svelte';
  import KitDetail from './lib/KitDetail.svelte';
  import Toasts from './lib/Toasts.svelte';
  import { t, formatTime, lang } from './lib/i18n.svelte';
  import { createHistory } from './lib/history';

  start();
  $effect(() => {
    // Local package replacements are independent of the release feed and its schedule.
    const timer = setInterval(checkInstalledDesktop, 60_000);
    return () => clearInterval(timer);
  });
  // The tray menu follows the language, the branding and what furnish is doing.
  $effect(() => syncTray(trayTexts()));

  const view = $derived(store.view);
  // By key, so collections using the same category share one entry; named as the first
  // collection that translates it names it, and sorted by that name.
  const categories = $derived(
    [
      ...new Set([
        ...store.apps.filter((a) => appStatus(a.key) !== 'unavailable').flatMap((a) => a.categories),
        ...store.kits.flatMap((k) => k.categories),
      ]),
    ]
      .map((key) => ({ key, name: categoryName(key) }))
      .sort((a, b) => a.name.localeCompare(b.name, lang())),
  );
  const count = (page: 'installed' | 'updates') => visibleApps({ page }, '').length;
  const checking = $derived(store.autoUpdate.running || store.scan.running);
  const failedChecks = $derived(Object.keys(store.statusErrors).length);
  const partialResults = $derived(failedChecks > 0 || !!store.autoUpdate.error || store.catalogError);
  const activity = $derived(Object.keys(store.busy).length + store.queue.length);
  const apps = $derived(visibleApps(view, store.query));
  const kits = $derived(visibleKits(view, store.query));
  const kit = $derived(view.page === 'kit' ? store.kits.find((k) => k.key === view.key) : undefined);
  // On Discover, before the full catalog: what the collections recommend for this user.
  const showRecommendations = $derived(view.page === 'discover' && !store.query);
  const featuredLimit = 4;
  const recommendedKits = $derived(showRecommendations ? kits.filter(kit => kit.recommended).slice(0, featuredLimit) : []);
  const recommendedApps = $derived(showRecommendations ? apps.filter(app => app.recommended).slice(0, featuredLimit - recommendedKits.length) : []);
  const remainingApps = $derived(apps.filter(app => !recommendedApps.includes(app)));
  const remainingKits = $derived(kits.filter(kit => !recommendedKits.includes(kit)));
  const compact = $derived(view.page === 'installed' || view.page === 'updates');
  const detail = $derived(view.page === 'app' ? store.apps.find((a) => a.key === view.key) : undefined);

  const titles = { discover: 'nav.discover', installed: 'nav.installed', updates: 'nav.updates' } as const;
  const title = $derived(
    view.page === 'category' ? categoryName(view.name) : view.page in titles ? t(titles[view.page as keyof typeof titles]) : '',
  );

  function go(next: View) {
    store.view = next;
  }
  // Back and forward like a browser: thumb buttons (3 back, 4 forward), Alt+Left/Right, Cmd+[ / ].
  let main: HTMLElement;
  let content: HTMLDivElement;
  type Visit = { view: View; top: number };
  const pages = createHistory<Visit>(visit => JSON.stringify(visit.view));
  let currentVisit: Visit | undefined;
  let restoring: Visit | undefined;
  $effect.pre(() => {
    const destination = $state.snapshot(store.view) as View;
    const first = !currentVisit;
    if (currentVisit && JSON.stringify(destination) === JSON.stringify(currentVisit.view)) return;
    if (currentVisit) currentVisit.top = main.scrollTop;
    const visit = restoring ?? { view: destination, top: 0 };
    restoring = undefined;
    currentVisit = visit;
    pages.seen(visit);
    if (first) return;
    const controller = new AbortController();
    tick().then(() => {
      if (controller.signal.aborted) return;
      main.scrollTop = visit.top;
      // Settings and detail panels grow as desktop data arrives. A webview clamps
      // scrollTop to the currently available height, so retry while it grows.
      if (main.scrollTop + 1 < visit.top) {
        const observer = new ResizeObserver(() => {
          main.scrollTop = visit.top;
          if (main.scrollTop + 1 >= visit.top) controller.abort();
        });
        observer.observe(content);
        const cancel = () => controller.abort();
        for (const event of ['pointerdown', 'wheel', 'keydown']) {
          window.addEventListener(event, cancel, { signal: controller.signal, capture: true, passive: true });
        }
        controller.signal.addEventListener('abort', () => observer.disconnect(), { once: true });
      }
      const heading = main.querySelector('h1');
      if (heading && !document.querySelector('dialog[open]')) {
        heading.tabIndex = -1;
        heading.focus({ preventScroll: true });
      }
    });
    return () => controller.abort();
  });
  function step(where: 'back' | 'forward') {
    const next = where === 'back' ? pages.back() : pages.forward();
    if (next) {
      restoring = next;
      store.view = next.view;
    }
  }
  function backTo(destination: View) {
    if (JSON.stringify(pages.previous()?.view) === JSON.stringify(destination)) step('back');
    else go(destination);
  }
  function onMouse(e: MouseEvent) {
    if (e.button !== 3 && e.button !== 4) return;
    e.preventDefault(); // the webview's own history is empty; keep it from trying
    if (e.type === 'mouseup') step(e.button === 3 ? 'back' : 'forward');
  }
  function onKey(e: KeyboardEvent) {
    const alt = e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey;
    const cmd = e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
    const where =
      (alt && e.key === 'ArrowLeft') || (cmd && e.key === '[') ? 'back'
      : (alt && e.key === 'ArrowRight') || (cmd && e.key === ']') ? 'forward'
      : null;
    if (!where) return;
    e.preventDefault();
    step(where);
  }
  // The install button keeps one width whatever it says; measure that width from this
  // language's labels in the system font (see the sizer below and ActionButton.svelte).
  let actionWidth = $state(0);
  let actionLargeWidth = $state(0);
  $effect(() => {
    const root = document.documentElement.style;
    if (actionWidth) root.setProperty('--action-width', `${actionWidth}px`);
    if (actionLargeWidth) root.setProperty('--action-large-width', `${actionLargeWidth}px`);
  });
  const idleLabels = $derived([t('common.install'), t('common.update'), t('common.uninstall'), t('action.answerFirst')]);
  const busyLabels = $derived([
    t('common.installing'), t('common.updating'), t('common.uninstalling'), t('action.stopping'), `${t('common.queued')} · 99`,
  ]);

  const isActive = (page: string, name?: string) =>
    view.page === page && (name === undefined || (view.page === 'category' && view.name === name));
</script>

<svelte:window onfocus={checkInstalledDesktop} onmousedown={onMouse} onmouseup={onMouse} onkeydown={onKey} />

{#snippet sizer()}
  {#each idleLabels as label}<span>{label}</span>{/each}
  {#each busyLabels as label}<span class="busy">{label}</span>{/each}
{/snippet}
<div class="sizer" aria-hidden="true" bind:clientWidth={actionWidth}>{@render sizer()}</div>
<div class="sizer large" aria-hidden="true" bind:clientWidth={actionLargeWidth}>{@render sizer()}</div>

<div class="shell" inert={store.desktopRestart === 'launching'}>
  <nav aria-label={t('nav.sections')}>
    <button class="brand" onclick={() => go({ page: 'discover' })} aria-label={t('nav.home', { brand: store.brand.name })}>
      <img src={store.brand.logo ?? logo} alt="" width="34" height="34" />
      <span>{store.brand.name}</span>
    </button>

    <button aria-current={isActive('discover') ? 'page' : undefined} onclick={() => go({ page: 'discover' })}>{t('nav.discover')}</button>
    <button aria-current={isActive('installed') ? 'page' : undefined} onclick={() => go({ page: 'installed' })}>
      {t('nav.installed')} <span class="count">{count('installed') || ''}</span>
    </button>
    <button aria-current={isActive('updates') ? 'page' : undefined} onclick={() => go({ page: 'updates' })}>
      {t('nav.updates')}
      {#if count('updates')}<span class="count pending">{count('updates')}</span>{/if}
    </button>

    <button aria-current={isActive('activity') ? 'page' : undefined} onclick={() => go({ page: 'activity' })}>
      {t('nav.activity')}
      {#if activity}<span class="count running">{activity}</span>{/if}
    </button>

    {#if categories.length}
      <div class="categories">
        <h2>{t('nav.categories')}</h2>
        {#each categories as { key, name } (key)}
          <button aria-current={isActive('category', key) ? 'page' : undefined} onclick={() => go({ page: 'category', name: key })}>{name}</button>
        {/each}
      </div>
    {/if}

    <div class="spacer"></div>
    <button aria-current={isActive('collections') ? 'page' : undefined} onclick={() => go({ page: 'collections' })}>
      {t('nav.collections')}
      {#if collectionUpdates()}
        <span class="count pending" role="img" aria-label={t('nav.collectionUpdates', { n: collectionUpdates() })}>{collectionUpdates()}</span>
      {/if}
    </button>
    <button aria-current={isActive('settings') ? 'page' : undefined} onclick={() => go({ page: 'settings' })}>
      {t('nav.settings')}
      {#if !store.installedDesktop && store.selfUpdate?.available && store.selfUpdate.mode !== 'off'}
        <span class="dot" role="img" aria-label={t('nav.selfUpdate', { brand: store.brand.name })} title={t('nav.selfUpdate', { brand: store.brand.name })}></span>
      {/if}
    </button>
  </nav>

  <main bind:this={main}>
    <div bind:this={content}>
    {#if store.installedDesktop && desktopIdle()}
      {@const installed = store.installedDesktop}
      <div class="restart-banner" data-testid="desktop-restart" role="status">
        <span>{t('desktop.installed', { brand: store.brand.name, version: installed.version })}{#if installed.revision} <code title={installed.revision}>{installed.revision.slice(0, 12)}</code>{/if}</span>
        <button class="primary" disabled={store.desktopRestart !== 'idle'} onclick={restartInstalledDesktop}>{t('desktop.restart', { brand: store.brand.name })}</button>
      </div>
    {/if}
    <SignInBanner />
    {#if view.page === 'collections'}
      <Collections />
    {:else if view.page === 'settings'}
      <Settings />
    {:else if view.page === 'activity'}
      <Activity onopen={(key) => go({ page: 'app', key, back: view })} />
    {:else if view.page === 'kit' && kit}
      <KitDetail
        {kit}
        onback={() => view.page === 'kit' && backTo(view.back)}
        onopen={(key) => go({ page: 'app', key, back: view })}
      />
    {:else if view.page === 'app' && detail}
      <AppDetail app={detail} onback={() => view.page === 'app' && backTo(view.back)} />
    {:else}
      <div class="top">
        <h1>{title}</h1>
        {#if view.page === 'updates'}
          <div class="check">
            <button class="primary" disabled={checking || store.loading || !availableAppUpdates().length} onclick={updateAllApps}>
              {t('common.updateAll')}
            </button>
            <button class="secondary" disabled={checking || store.loading} onclick={checkForUpdates}>
              {checking ? t('catalog.checking') : t('catalog.checkForUpdates')}
            </button>
            <span class="muted" aria-live="polite">
              {#if store.scan.running}
                {t('catalog.scanProgress', { done: store.scan.checked, total: store.scan.total })}
              {:else if store.autoUpdate.running}
                {t('settings.checking')}
              {:else if failedChecks}
                {t('catalog.scanFailures', { n: failedChecks })}
              {/if}
              {#if store.autoUpdate.postponed}
                <span class="postponed">{t('catalog.postponed', {
                  // The backend says "measuring …" until it has a first sample (src/load.rs).
                  reason: store.autoUpdate.postponed.startsWith('measuring')
                    ? t('catalog.measuring')
                    : t('catalog.busy', { load: store.autoUpdate.postponed }),
                })}</span>
              {:else if !checking && (store.autoUpdate.error || store.catalogError)}{t('catalog.lastCheckFailed')}{:else if !checking && !failedChecks && store.autoUpdate.lastAt}{t('catalog.lastChecked', {
                  time: formatTime(store.autoUpdate.lastAt),
                })}{/if}
            </span>
          </div>
        {/if}
        {#if store.apps.length || store.kits.length}
          <input
            class="search"
            type="search"
            placeholder={t('catalog.search')}
            aria-label={t('catalog.search')}
            bind:value={store.query}
          />
        {/if}
      </div>

      {#if store.loading}
        <p class="empty">{t('catalog.loading')}</p>
      {:else if !store.collections.length && store.settingUpCollections}
        <p class="empty" role="status">{t('catalog.settingUp')}</p>
      {:else if !store.collections.length}
        <div class="empty">
          <h2>{t('catalog.noCollections')}</h2>
          <p>{t('catalog.noCollectionsText')}</p>
          <button class="primary" onclick={() => go({ page: 'collections' })}>{t('catalog.addCollection')}</button>
        </div>
      {:else if !apps.length && !kits.length}
        {#if store.query}
          <p class="empty">{t('catalog.noMatch', { query: store.query })}</p>
        {:else if view.page !== 'updates'}
          <p class="empty">{t('catalog.empty')}</p>
        {:else if !checking && store.scan.complete}
          <p class="empty">{partialResults ? t('catalog.noUpdatesFound') : t('catalog.upToDate', { brand: store.brand.name })}</p>
        {/if}
      {:else}
        {#if recommendedKits.length || recommendedApps.length}
          <h2 class="row-title">{t('catalog.recommended')}</h2>
          <div class="grid recommended">
            {#each recommendedKits as k (k.key)}
              <KitCard kit={k} onopen={() => go({ page: 'kit', key: k.key, back: view })} />
            {/each}
            {#each recommendedApps as app (app.key)}
              <AppCard {app} onopen={() => go({ page: 'app', key: app.key, back: view })} />
            {/each}
          </div>
          {#if remainingApps.length || remainingKits.length}<h2 class="row-title">{t('catalog.more')}</h2>{/if}
        {/if}
        <div class="grid" class:compact>
          {#each remainingKits as k (k.key)}
            <KitCard kit={k} onopen={() => go({ page: 'kit', key: k.key, back: view })} />
          {/each}
          {#each remainingApps as app (app.key)}
            <AppCard {app} {compact} onopen={() => go({ page: 'app', key: app.key, back: view })} />
          {/each}
        </div>
      {/if}
    {/if}
    </div>
  </main>
</div>

<Toasts />
<DepsDialog />
<QuestionsDialog />
<LinkDialog />

<style>
  .restart-banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem 1rem;
    padding: 0.8rem 1rem;
    margin-bottom: 1.5rem;
    border-radius: 12px;
    background: var(--highlight-soft);
    color: var(--ink);
    font-size: 0.92rem;
  }
  .restart-banner code {
    color: var(--ink-soft);
    font-family: var(--font-mono);
    font-size: 0.85em;
  }

  /* Paddings, border and font match ActionButton.svelte's .action, .busy and .large. */
  .sizer {
    position: absolute;
    visibility: hidden;
    pointer-events: none;
    display: grid;
    font: 600 0.85rem/1 var(--font-body);
  }
  .sizer span {
    grid-area: 1 / 1;
    padding: 0 1rem;
    border: 1px solid transparent;
    white-space: nowrap;
  }
  .sizer span.busy {
    padding-right: 2.3rem;
  }
  .sizer.large {
    font-size: 1rem;
  }
  .sizer.large span.busy {
    padding-right: 2.9rem;
  }
  .shell {
    display: grid;
    grid-template-columns: 14.5rem minmax(0, 1fr);
    height: 100vh;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: .25rem;
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
    letter-spacing: -0.03em;
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
    min-height: 2.5rem;
    padding: .5rem .8rem;
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
  nav button[aria-current="page"] {
    background: var(--surface);
    box-shadow: inset 3px 0 var(--accent);
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
  /* A new version of furnish: a quiet mark, not a count. */
  .dot {
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 50%;
    background: var(--accent);
  }
  .categories { display: flex; flex-direction: column; gap: .25rem; min-height: 0; overflow-y: auto; }
  nav > button, .categories > button { flex: none; }
  .grid.compact { grid-template-columns: minmax(0, 1fr); gap: .5rem; }
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
    font: var(--page-heading);
    letter-spacing: -0.03em;
  }
  .check {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.75rem;
    margin-right: auto;
  }
  .check .muted {
    font-size: 0.82rem;
    color: var(--muted);
  }
  .postponed {
    font-weight: 650;
    color: var(--highlight-ink);
  }
  .search {
    width: min(20rem, 100%);
  }
  .row-title {
    margin: 0 0 0.9rem;
    font: 650 1.05rem var(--font-display);
  }
  .grid.recommended {
    margin-bottom: 2rem;
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
  @media (max-height: 580px) {
    nav { padding-block: .65rem; }
    nav button.brand { margin-bottom: .4rem; }
    nav h2 { margin-block: .6rem .25rem; }
  }
  @media (max-width: 760px) {
    .shell {
      grid-template-columns: 11rem minmax(0, 1fr);
    }
    main {
      padding: 1.5rem 1.25rem 3rem;
    }
  }
</style>

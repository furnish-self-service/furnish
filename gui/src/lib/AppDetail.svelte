<script lang="ts">
  import CollectionAvatar from './CollectionAvatar.svelte';
  import type { AppInfo } from './api';
  import { store, appName, appDescription, categoryName, uninstall, install, reinstall, defaultDependencies, setAutoUpdate, isLocked, isPending, installPrevious, installCurrent, externalStatus, requiredUpdate } from './state.svelte';
  import { describeNeeds, describeRequired, unmet } from './required';
  import Options from './Options.svelte';
  import DependencyList from './DependencyList.svelte';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';
  import { highlightYaml } from './highlight';
  import Plan from './Plan.svelte';
  import { t, formatDate, lang } from './i18n.svelte';
  import External from './External.svelte';

  let { app, onback }: { app: AppInfo; onback: () => void } = $props();
  let confirming = $state(false);
  let logList = $state<HTMLOListElement>();
  let planOpen = $state(false);
  // Dependencies to install along with the app; the list below lets users change optional ones.
  let chosen = $state<string[]>([]);
  $effect.pre(() => {
    chosen = defaultDependencies(app);
  });
  const autoUpdate = $derived(!store.settings?.noAutoUpdate.includes(app.key));

  const lines = $derived(store.logs[app.key] ?? []);
  const busy = $derived(!!store.busy[app.key]);
  // Follow the log while it grows.
  $effect(() => {
    lines.length;
    if (logList) logList.scrollTop = logList.scrollHeight;
  });

  const status = $derived(store.status[app.key]);
  const installed = $derived(status === 'installed' || status === 'needsUpdate');
  const required = $derived(store.required[app.key]);
  // A security update the collection requires: installs right away, automatic updates or not.
  const securityUpdate = $derived(requiredUpdate(app.key));
  // No acceptable version is offered: what the collection needs and what there is.
  const needs = $derived(unmet(required) ? describeRequired(lang(), required) : null);
  const statusText = $derived.by(() => {
    const what = store.reasons[app.key] ?? t('common.missingTool');
    switch (status) {
      case 'installed':
        if (store.external[app.key]) return externalStatus(store.external[app.key]);
        if (needs && required.installed) return t('app.status.unacceptable', { installed: required.installed, needs });
        return t('app.status.installed');
      case 'needsUpdate': return securityUpdate ? t('app.status.securityUpdate') : t('app.status.needsUpdate');
      case 'notInstalled': return t('app.status.notInstalled');
      case 'unavailable': return t('app.status.unavailable');
      case 'blocked':
        if (needs) return needs;
        return app.dependencies.length ? t('app.status.blockedDeps', { what, app: appName(app) }) : t('app.status.blocked', { what });
      case 'error': return t('app.status.error');
      default: return t('app.status.checking');
    }
  });
</script>

<section class="detail">
  <button class="back" onclick={onback}>{t('common.back')}</button>

  <header>
    <AppIcon {app} size={96} />
    <div class="heading">
      <h1>{appName(app)}</h1>
      <p class="meta">{app.categories.map((c) => categoryName(c, app.collection)).join(' · ')}{app.categories.length ? t('common.metaSeparator') : ''}{t('common.from')} <CollectionAvatar name={app.collection} size={20} /> {app.collection}</p>
      <p class="status" class:installed={status === 'installed' && !needs} class:update={status === 'needsUpdate' || status === 'blocked' || !!needs} class:required={securityUpdate}>{statusText}</p>
      {#if securityUpdate && required.installed}
        <p class="required-detail">{t('app.requiredUpdate', { collection: app.collection, needs: describeNeeds(lang(), required.needs), installed: required.installed, target: required.target ?? '' })}</p>
      {/if}
      {#if required?.unknown}
        <p class="required-unknown">{store.external[app.key]?.path
          ? t('app.requiredUnknownAt', { brand: store.brand.name, collection: app.collection, path: store.external[app.key].path ?? '' })
          : t('app.requiredUnknown', { brand: store.brand.name, collection: app.collection })}</p>
      {/if}
    </div>
    <div class="cta">
      {#if status === 'installed' && confirming}
        <p class="confirm">{t('app.confirmUninstall', { app: appName(app) })}</p>
        <div class="row">
          <button class="danger" onclick={() => { confirming = false; uninstall(app); }}>{t('common.uninstall')}</button>
          <button class="secondary" onclick={() => (confirming = false)}>{t('common.cancel')}</button>
        </div>
      {:else if status === 'installed' && !store.busy[app.key] && !store.queue.some((q) => q.key === app.key)}
        {#if store.external[app.key]}
          <!-- Installed outside furnish: removing it is done there. -->
          <External external={store.external[app.key]} />
        {:else if store.unremovable[app.key]}
          <!-- A resource can't remove what it set up, and the collection gives no removal steps. -->
          <p class="unremovable">{t('app.cantUninstall', { collection: app.collection })}</p>
        {:else}
          <button class="secondary large" onclick={() => (confirming = true)}>{t('common.uninstall')}</button>
        {/if}
      {:else}
        <ActionButton {app} large oninstall={() => install(app, chosen)} />
      {/if}
      {#if installed && !store.external[app.key]}
        <label class="auto">
          <input
            type="checkbox"
            checked={autoUpdate}
            disabled={isLocked('noAutoUpdate')}
            onchange={(e) => setAutoUpdate(app.key, e.currentTarget.checked)}
          />
          {isLocked('noAutoUpdate') ? t('app.autoUpdateLocked') : t('app.autoUpdate')}
        </label>
        {#if securityUpdate}<p class="auto-note">{t('app.autoUpdateRequired', { collection: app.collection })}</p>{/if}
      {/if}
    </div>
  </header>

  <p class="description">{appDescription(app) || t('common.noDescription')}</p>

  {#if app.held}
    <div class="differ" role="status">
      <span>{t('app.held', { app: appName(app), date: formatDate(app.held.date) })}</span>
      <button class="secondary" disabled={isPending(app.key)} onclick={() => installCurrent(app)}>
        {isPending(app.key) ? t('common.installing') : t('app.tryCurrent')}
      </button>
    </div>
  {:else if store.failed[app.key]?.previous}
    {@const previous = store.failed[app.key].previous!}
    <div class="differ" role="status">
      <span>{t('app.previousWorks', { app: appName(app), date: formatDate(previous.date) })}</span>
      <button class="secondary" disabled={isPending(app.key)} onclick={() => installPrevious(app)}>
        {isPending(app.key) ? t('common.installing') : t('app.installPrevious', { date: formatDate(previous.date) })}
      </button>
    </div>
  {/if}

  {#if installed && app.optionsDiffer}
    <div class="differ" role="status">
      <span>{t('app.optionsDiffer', { app: appName(app) })}</span>
      <button class="secondary" disabled={isPending(app.key)} onclick={() => reinstall(app)}>
        {isPending(app.key) ? t('app.reinstalling') : t('app.reinstall')}
      </button>
    </div>
  {/if}

  {#if app.options.length}
    <h2>{t('app.options')}</h2>
    <p class="lead">
      {installed ? t('app.optionsOnReinstall') : t('app.optionsOnInstall', { app: appName(app) })}
      {t('app.optionsFollow', { collection: app.collection })}
    </p>
    <Options target={app.key} options={app.options} scope="app" />
  {/if}

  {#if app.dependencies.length}
    <h2>{t('app.dependencies')}</h2>
    <p class="lead">
      {app.dependencies.some((d) => !d.required)
        ? t('app.dependenciesLeadOptional', { app: appName(app) })
        : t('app.dependenciesLead', { app: appName(app) })}
    </p>
    <DependencyList {app} bind:chosen />
  {/if}

  {#if app.resources.length}
    <h2>{t('app.setsUp')}</h2>
    <ul class="resources">
      {#each app.resources as resource}
        <li>{resource.name}</li>
      {/each}
    </ul>
  {/if}

  <!-- Expert details: collapsed by default, one click to reveal. -->
  <details class="expert" bind:open={planOpen}>
    <summary>{t('app.steps')} <span class="hint">{t('app.stepsHint')}</span></summary>
    <!-- Mounted only while open: the preview asks DSC, which takes a moment. -->
    {#if planOpen}<Plan {app} />{/if}
  </details>

  <details class="expert">
    <summary>
      {t('app.log')}
      {#if busy}<span class="live">{t('app.live')}</span>{:else if lines.length}<span class="hint">{t('app.lines', { n: lines.length })}</span>{/if}
    </summary>
    {#if lines.length}
      <ol class="panel log" bind:this={logList}>
        {#each lines as line}
          <li class={line.level}><span class="level">{line.level}</span><span>{line.message}</span></li>
        {/each}
      </ol>
    {:else}
      <p class="empty">{t('app.logEmpty')}</p>
    {/if}
  </details>

  <details class="expert">
    <summary>{t('app.configuration')} <span class="hint">apps/{app.id}.dsc.yaml</span></summary>
    <!-- highlightYaml escapes the source before adding markup. -->
    <pre class="panel code">{@html highlightYaml(app.source)}</pre>
  </details>
</section>

<style>
  .detail {
    max-width: 52rem;
  }
  .back {
    margin-bottom: 1.5rem;
    padding: 0.3rem 0;
    border: 0;
    background: none;
    color: var(--accent-ink);
    font: 600 0.9rem var(--font-body);
    cursor: pointer;
  }
  header {
    display: flex;
    gap: 1.5rem;
    align-items: center;
    flex-wrap: wrap;
  }
  .heading {
    flex: 1;
    min-width: 14rem;
  }
  h1 {
    margin: 0;
    font: 700 2.4rem/1.05 var(--font-display);
    letter-spacing: -0.02em;
  }
  .meta {
    margin: 0.4rem 0 0;
    color: var(--muted);
  }
  .status {
    margin: 0.6rem 0 0;
    font-weight: 600;
    font-size: 0.9rem;
  }
  .status.installed {
    color: var(--accent-ink);
  }
  .status.update {
    color: var(--highlight-ink);
  }
  .status.required {
    color: var(--danger);
  }
  .required-unknown {
    margin: 4px 0 0;
    color: var(--muted);
    font-size: 0.9em;
  }
  .required-detail {
    margin: 0.3rem 0 0;
    font-size: 0.85rem;
    color: var(--ink-soft);
  }
  .auto-note {
    max-width: 16rem;
    margin: 0;
    font-size: 0.8rem;
    color: var(--muted);
    text-align: right;
  }
  .cta {
    display: grid;
    gap: 0.5rem;
    justify-items: end;
  }
  .confirm {
    margin: 0;
    font-weight: 600;
  }
  .unremovable {
    max-width: 16rem;
    margin: 0;
    font-size: 0.85rem;
    color: var(--ink-soft);
    text-align: right;
  }
  .row {
    display: flex;
    gap: 0.5rem;
  }
  /* Same slot size as ActionButton's large variant, so swapping them never moves anything. */
  .large {
    box-sizing: border-box;
    width: 10.5rem;
    height: 2.75rem;
    padding: 0 1rem;
    font-size: 1rem;
    border-radius: 999px;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.85rem;
    color: var(--ink-soft);
    cursor: pointer;
  }
  .auto input {
    accent-color: var(--accent);
  }
  .differ {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem 1.5rem;
    margin: -0.5rem 0 1.5rem;
    padding: 0.8rem 1rem;
    border-radius: 12px;
    background: var(--hover);
    color: var(--highlight-ink);
    font-weight: 600;
    font-size: 0.92rem;
  }
  .lead {
    margin: -0.4rem 0 0.6rem;
    font-size: 0.85rem;
    color: var(--muted);
  }
  .description {
    margin: 2rem 0;
    font-size: 1.05rem;
    line-height: 1.6;
    color: var(--ink-soft);
  }
  h2 {
    font: 650 1rem var(--font-display);
    margin: 1.75rem 0 0.75rem;
  }
  .resources {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  .resources li {
    font-weight: 600;
    padding: 0.7rem 0;
    border-bottom: 1px solid var(--line);
  }
  .expert {
    margin-top: 1rem;
    border-top: 1px solid var(--line);
    padding-top: 0.75rem;
  }
  .expert:first-of-type {
    margin-top: 2.5rem;
  }
  summary {
    cursor: pointer;
    font: 650 0.95rem var(--font-display);
    color: var(--ink-soft);
    padding: 0.25rem 0;
  }
  summary:hover {
    color: var(--ink);
  }
  .hint {
    margin-left: 0.5rem;
    font: 400 0.78rem var(--font-mono);
    color: var(--muted);
  }
  .live {
    margin-left: 0.5rem;
    padding: 0.05rem 0.45rem;
    border-radius: 999px;
    font: 700 0.7rem var(--font-body);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--on-accent);
    background: var(--accent);
  }
  .empty {
    color: var(--muted);
    font-size: 0.9rem;
  }
  /* Terminal-style panel, dark in both themes. */
  .panel {
    margin: 0.75rem 0 0;
    padding: 0.9rem 1rem;
    max-height: 22rem;
    overflow: auto;
    border-radius: 12px;
    background: #0e1d21;
    color: #d5e5e3;
    font: 0.8rem/1.55 var(--font-mono);
  }
  .log {
    list-style: none;
  }
  .log li {
    display: flex;
    gap: 0.9rem;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .level {
    flex: none;
    width: 3.6rem;
    text-align: right;
    opacity: 0.85;
  }
  .log .error {
    color: #ff8f86;
  }
  .log .warn {
    color: #f6c453;
  }
  .log .info {
    color: #7fdccf;
  }
  .log .output {
    color: #9db2b5;
  }
  .code :global(.y-key) {
    color: #7fdccf;
  }
  .code :global(.y-string) {
    color: #f0b98d;
  }
  .code :global(.y-expr) {
    color: #c8a8ff;
  }
  .code :global(.y-comment) {
    color: #6d878c;
    font-style: italic;
  }
</style>

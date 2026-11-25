<script lang="ts">
  import Description from './Description.svelte';
  import ActionLinks from './ActionLinks.svelte';
  import { tick } from 'svelte';
  import CollectionAvatar from './CollectionAvatar.svelte';
  import type { AppInfo } from './api';
  import { appStatus, store, appName, appDescription, categoryName, uninstall, install, reinstall, defaultDependencies, setAutoUpdate, isLocked, isPending, installPrevious, installCurrent, externalStatus, requiredUpdate } from './state.svelte';
  import { describeNeeds, describeRequired, unmet } from './required';
  import Options from './Options.svelte';
  import OptionChanges from './OptionChanges.svelte';
  import DependencyList from './DependencyList.svelte';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';
  import CheckStatusButton from './CheckStatusButton.svelte';
  import { highlightYaml } from './highlight';
  import RunLog from './RunLog.svelte';
  import Plan from './Plan.svelte';
  import Versions from './Versions.svelte';
  import { t, formatDate, lang, terror } from './i18n.svelte';
  import RemovalImpact from './RemovalImpact.svelte';
  import External from './External.svelte';

  let { app, onback }: { app: AppInfo; onback: () => void } = $props();
  let confirming = $state(false);
  let planOpen = $state(false);
  let planPanel: HTMLDetailsElement;
  let plan = $state<ReturnType<typeof Plan>>();
  let previewCount = $state<number | null>(null);
  async function openPreview() {
    planOpen = true;
    await tick();
    plan?.showInstallPreview();
    planPanel.querySelector('summary')?.focus({ preventScroll: true });
    planPanel.scrollIntoView({ block: 'start' });
  }
  let versionsOpen = $state(false);
  // People may choose a version of the collection's apps (setting chooseVersion).
  const canChoose = $derived(store.collections.find((c) => c.name === app.collection)?.settings.settings.chooseVersion ?? false);
  // Dependencies to install along with the app; the list below lets users change optional ones.
  let chosen = $state<string[]>([]);
  $effect.pre(() => {
    chosen = defaultDependencies(app);
  });
  const autoUpdate = $derived(!store.settings?.noAutoUpdate.includes(app.key));

  const check = $derived(store.checks[app.key]);
  const status = $derived(appStatus(app.key));
  const run = $derived(store.runs[app.key]);
  const checkError = $derived(store.statusErrors[app.key]);
  const checkedAt = $derived(check?.checkedAt);
  const installed = $derived(status === 'installed' || status === 'needsUpdate');
  const required = $derived(check?.required);
  // A security update the collection requires: installs right away, automatic updates or not.
  const securityUpdate = $derived(requiredUpdate(app.key));
  // No acceptable version is offered: what the collection needs and what there is.
  const needs = $derived(unmet(required) ? describeRequired(lang(), required) : null);
  const statusText = $derived.by(() => {
    const what = check?.reason ?? t('common.missingTool');
    switch (status) {
      case 'installed':
        if (check?.external) return externalStatus(check?.external);
        if (needs && required?.installed) return t('app.status.unacceptable', { installed: required.installed, needs });
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
  <div class="toolbar">
    <button class="back" onclick={onback}>{t('common.back')}</button>
    <ActionLinks kind="app" collection={app.collection} id={app.id} name={appName(app)} />
  </div>

  <header>
    <AppIcon {app} size={96} />
    <div class="heading">
      <h1>{appName(app)}</h1>
      <p class="meta">{app.categories.map((c) => categoryName(c, app.collection)).join(' · ')}{app.categories.length ? t('common.metaSeparator') : ''}{t('common.from')} <CollectionAvatar name={app.collection} size={20} /> {app.collection}</p>
      <p class="status" class:installed={status === 'installed' && !needs} class:update={status === 'needsUpdate' || status === 'blocked' || !!needs} class:required={securityUpdate}>
        {checkError !== undefined && status !== 'error' ? t('status.lastKnown', { state: statusText }) : statusText}
      </p>
      {#if checkError !== undefined}
        <p class="check-error" role="status">{#if status !== 'error'}{t('app.status.error')}{/if}
          {#if checkError}<span>{terror(checkError)}</span>{/if}
        </p>
      {/if}
      <div class="status-check">
        <CheckStatusButton appKey={app.key} />
        {#if checkedAt}
          <span>{t('status.checkedAt', { time: new Date(checkedAt).toLocaleString(lang(), { dateStyle: 'medium', timeStyle: 'short' }) })}</span>
        {/if}
      </div>
      {#if securityUpdate && required?.installed && checkError === undefined}
        <p class="required-detail">{t('app.requiredUpdate', { collection: app.collection, needs: describeNeeds(lang(), required.needs), installed: required.installed, target: required.target ?? '' })}</p>
      {/if}
      {#if required?.unknown}
        <p class="required-unknown">{check?.external?.path
          ? t('app.requiredUnknownAt', { brand: store.brand.name, collection: app.collection, path: check?.external.path ?? '' })
          : t('app.requiredUnknown', { brand: store.brand.name, collection: app.collection })}</p>
      {/if}
    </div>
    <div class="cta">
      {#if status === 'installed' && confirming && checkError === undefined}
        <p class="confirm">{t('app.confirmUninstall', { app: appName(app) })}</p>
        <RemovalImpact keys={[app.key]} />
        <div class="row">
          <button class="danger" onclick={() => { confirming = false; return uninstall(app); }}>{t('common.uninstall')}</button>
          <button class="secondary" onclick={() => (confirming = false)}>{t('common.cancel')}</button>
        </div>
      {:else if status === 'installed' && !store.busy[app.key] && !store.queue.some((q) => q.key === app.key)}
        {#if check?.external}
          <!-- Installed outside furnish: removing it is done there. -->
          <External external={check?.external} />
        {:else if check?.removable === false}
          <!-- A resource can't remove what it set up, and the collection gives no removal steps. -->
          <p class="unremovable">{t('app.cantUninstall', { collection: app.collection })}</p>
        {:else if checkError === undefined}
          <button class="secondary large" onclick={() => (confirming = true)}>{t('common.uninstall')}</button>
        {/if}
      {:else if checkError === undefined || isPending(app.key)}
        <ActionButton {app} large oninstall={() => install(app, chosen)} />
      {/if}
      {#if status === 'needsUpdate' && !isPending(app.key) && checkError === undefined}
        <button class="secondary" onclick={openPreview}>{t('app.previewChanges')}{#if planOpen && previewCount !== null}{' '}({previewCount}){/if}</button>
      {/if}
      {#if installed && !check?.external}
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

  {#if check?.runningApps?.length}
    <p class="meta close-apps">{t('queue.closeApps', {names: check?.runningApps.join(', ')})}</p>
  {/if}
  <div class="description"><Description text={appDescription(app) || t('common.noDescription')} collection={app.collection} /></div>

  {#if app.held?.chosen}
    <div class="differ" role="status">
      <span>{t(app.held.newer ? 'app.heldChosenNewer' : 'app.heldChosen', { app: appName(app), date: formatDate(app.held.date) })}</span>
      <button class="secondary" disabled={isPending(app.key)} onclick={() => installCurrent(app)}>
        {isPending(app.key) ? t('common.installing') : t('app.installCurrent')}
      </button>
    </div>
  {:else if app.held}
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

  {#if !planOpen && installed && run?.requiresNewSession && (run.result === 'done' || run.result === 'fallback')}
    <p class="differ" role="status">{t('session.after')}</p>
  {/if}

  {#if installed && Object.keys(app.optionChanges ?? {}).length}
    <div class="differ" role="status">
      <span>{t('app.optionsDiffer', { app: appName(app) })}</span>
      <button class="secondary" disabled={isPending(app.key)} onclick={() => reinstall(app)}>
        {isPending(app.key) ? t('app.reinstalling') : t('app.reinstall')}
      </button>
      <OptionChanges {app} />
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
  {#if canChoose && status !== 'unavailable'}
    <details class="expert" bind:open={versionsOpen}>
      <summary>{t('app.versions')} <span class="hint">{t('app.versionsHint')}</span></summary>
      <!-- Mounted only while open: listing the versions may download earlier files. -->
      {#if versionsOpen}<Versions {app} />{/if}
    </details>
  {/if}
  <details class="expert" bind:this={planPanel} bind:open={planOpen}>
    <summary>{t('app.steps')} <span class="hint">{t('app.stepsHint')}</span></summary>
    <!-- Mounted only while open: the preview asks DSC, which takes a moment. -->
    {#if planOpen}<Plan {app} bind:this={plan} oncount={(n) => previewCount = n} />{/if}
  </details>

  <RunLog lines={store.logs[app.key] ?? []} busy={!!store.busy[app.key]} />

  <details class="expert">
    <summary>{t('app.configuration')} <span class="hint">apps/{app.id}.dsc.yaml</span></summary>
    <!-- highlightYaml escapes the source before adding markup. -->
    <pre class="panel code">{@html highlightYaml(app.source)}</pre>
  </details>
</section>

<style>
  .close-apps { margin: 0; color: var(--ink-soft); font-size: .88rem; }
  .detail {
    max-width: 52rem;
  }
  .toolbar { display: flex; align-items: center; justify-content: space-between; margin-bottom: 1.5rem; }
  .back {
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
  .status-check {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    margin-top: 0.5rem;
    color: var(--muted);
    font-size: 0.82rem;
  }
  .check-error {
    margin: 0.4rem 0;
    color: var(--ink-soft);
    font-size: 0.9rem;
    overflow-wrap: anywhere;
  }
  .check-error span {
    display: block;
    white-space: pre-wrap;
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

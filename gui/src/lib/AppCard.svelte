<script lang="ts">
  import Description from './Description.svelte';
  import type { AppInfo } from './api';
  import { appStatus, store, appName, appDescription, categoryName, externalStatus, requiredUpdate } from './state.svelte';
  import { describeRequired, unmet } from './required';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';
  import CheckStatusButton from './CheckStatusButton.svelte';
  import CollectionAvatar from './CollectionAvatar.svelte';
  import { t, lang } from './i18n.svelte';

  let { app, onopen, compact = false }: { compact?: boolean; app: AppInfo; onopen: () => void } = $props();
  const check = $derived(store.checks[app.key]);
  const status = $derived(appStatus(app.key));
  const checkFailed = $derived(store.statusErrors[app.key] !== undefined);
  // No acceptable version is offered: what the collection needs and what there is.
  const needs = $derived(unmet(check?.required) ? describeRequired(lang(), check?.required) : null);
  const stateLabel = $derived.by(() => {
    if (needs && (status === 'installed' || status === 'blocked')) return needs;
    switch (status) {
      case 'installed': return check?.external ? externalStatus(check?.external) : t('common.installed');
      case 'needsUpdate': return requiredUpdate(app.key) ? t('common.securityUpdate') : t('common.updateAvailable');
      case 'notInstalled': return checkFailed ? t('app.status.notInstalled') : null;
      case 'unavailable': return checkFailed ? t('app.status.unavailable') : null;
      case 'blocked': return checkFailed ? t('action.needs', { what: check?.reason ?? t('common.missingTool') }) : null;
      default: return null;
    }
  });
</script>

<article class="card" class:compact>
  <button class="open" onclick={onopen} aria-label={t('common.open', { name: appName(app) })}></button>
  <header>
    <AppIcon {app} size={compact ? 36 : 56} />
    <div class="title">
      <h3>{appName(app)}</h3>
      {#if app.recommended && !compact}<span class="recommended">{t('card.recommended')}</span>{/if}
      <p class="meta">{app.categories.map((c) => categoryName(c, app.collection)).join(' · ') || app.collection}</p>
    </div>
  </header>
  {#if check?.runningApps?.length}
    <p class="meta close-apps">{t('queue.closeApps', {names: check?.runningApps.join(', ')})}</p>
  {/if}
  {#if !compact}<p class="description"><Description text={appDescription(app) || t('common.noDescription')} collection={app.collection} compact /></p>{/if}
  <footer>
    {#if stateLabel}
      <span class="state" class:installed={status === 'installed' && !needs && !checkFailed}
        class:update={status === 'needsUpdate' || !!needs} class:required={requiredUpdate(app.key)}
        class:needs={!!needs} title={needs ?? undefined}>
        {checkFailed ? t('status.lastKnown', { state: stateLabel }) : stateLabel}
      </span>
    {:else if checkFailed}
      <span class="state">{t('action.status')}</span>
    {:else}
      <span class="collection"><CollectionAvatar name={app.collection} /> {app.collection}</span>
    {/if}
    <!-- Uninstalling is only offered on the detail page, behind a confirmation. -->
    {#if checkFailed && !store.busy[app.key] && !store.queue.some((q) => q.key === app.key)}
      <CheckStatusButton appKey={app.key} />
    {:else if status !== 'installed' || store.busy[app.key] || store.queue.some((q) => q.key === app.key)}
      <ActionButton {app} />
    {/if}
  </footer>
</article>

<style>
  .close-apps { margin: 0; color: var(--ink-soft); font-size: .88rem; }
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
  .card.compact {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(40%, auto);
    align-items: center;
    padding: .75rem 1rem;
  }
  .compact header, .compact footer { min-width: 0; }
  .compact footer { flex-wrap: wrap; }
  .compact .close-apps { grid-column: 1 / -1; grid-row: 2; }
  .compact h3 { font-size: 1rem; }
  @media (max-width: 800px) {
    .card.compact { grid-template-columns: minmax(0, 1fr); }
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
  .required {
    color: var(--danger);
  }
  /* Can be long ("Needs 128.0.3 or later; winget offers 128.0.1"): wraps instead of widening. */
  .needs {
    align-items: flex-start;
    font-size: 0.78rem;
    line-height: 1.3;
  }
  .needs::before {
    flex: none;
    margin-top: 0.3rem;
  }
  .recommended {
    font-size: 0.72rem;
    font-weight: 700;
    color: var(--accent-ink);
  }
</style>

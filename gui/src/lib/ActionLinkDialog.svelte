<script lang="ts">
  import { untrack } from 'svelte';
  import * as api from './api';
  import type { ActionLink } from './action-link';
  import { appByKey, appName, checkStatus, defaultUninstall, dismissLink, eligibleForLink, isVerified, kitName,
    runActionLink, store } from './state.svelte';
  import { t, terror } from './i18n.svelte';
  import RemovalImpact from './RemovalImpact.svelte';

  let { link, url }: { link: ActionLink; url: string } = $props();
  let dialog: HTMLDialogElement;
  let missingCollection = $state(false);
  let initialSource: { url: string; revision: string } | undefined;
  let submitting = $state(false);
  let error = $state('');
  $effect(() => { dialog.showModal(); });

  const key = $derived(`${link.collection}/${link.id}`);
  const collection = $derived(store.collections.find(c => c.name === link.collection));
  const kit = $derived(link.kind === 'kit' ? store.kits.find(k => k.key === key) : undefined);
  const app = $derived(link.kind === 'app' ? appByKey(key) : undefined);
  const target = $derived(app ? appName(app) : kit ? kitName(kit) : link.id);
  const members = $derived(kit ? kit.items.filter(i => link.action !== 'install' || i.required || i.selected).map(i => i.key) : [key]);
  const failed = $derived(members.find(k => !appByKey(k) || store.statusErrors[k] !== undefined));
  let reviewing = $state(true);
  $effect(() => {
    if (store.loading || store.catalogError || missingCollection || error) return;
    if (!collection) {
      missingCollection = true;
      return;
    }
    if (initialSource && (initialSource.url !== collection.url || initialSource.revision !== collection.revision)) {
      error = t('link.changed');
      return;
    }
    initialSource = { url: collection.url, revision: collection.revision };
    if (!(app || kit)) {
      error = t('deps.notFound');
      return;
    }
    const keys = members.filter(k => appByKey(k));
    let cancelled = false;
    reviewing = true;
    untrack(async () => {
      for (const key of keys) {
        if (cancelled) return;
        await checkStatus(key, 'link-preview');
      }
      if (!cancelled) reviewing = false;
    });
    return () => { cancelled = true; };
  });
  const checking = $derived(reviewing || members.some(k => !isVerified(k)));
  const apps = $derived.by(() => {
    const keys = kit && link.action === 'uninstall' ? defaultUninstall(kit) : members;
    return keys.map(appByKey).filter((a): a is api.AppInfo => !!a && eligibleForLink(link.action, a));
  });
  const needsAnswers = $derived(link.action === 'install' && apps.some(app => app.unanswered.length));
  const ready = $derived(!missingCollection && !store.loading && !store.catalogError && !!collection && !!(app || kit)
    && !failed && !checking && apps.length > 0 && !error && store.desktopRestart !== 'launching');

  function collections() {
    dismissLink(url);
    store.view = { page: 'collections' };
  }

  async function confirm() {
    if (!ready || submitting || !collection) return;
    const request = store.link;
    const shown = apps;
    const source = collection;
    submitting = true;
    try {
      const current = (await api.listCollections()).find(c => c.name === link.collection);
      if (store.link !== request) return;
      if (!current || current.url !== source.url || current.revision !== source.revision
        || !ready || collection.url !== source.url || collection.revision !== source.revision
        || shown.length !== apps.length || shown.some((a, i) => a.key !== apps[i].key)) {
        error = t('link.changed');
        return;
      }
      if (runActionLink(link, apps, kit)) dismissLink(url);
      else error = t('link.changed');
    } catch (e) {
      error = terror(String(e));
    } finally {
      submitting = false;
    }
  }
</script>

<dialog bind:this={dialog} onclose={() => dismissLink(url)} aria-labelledby="action-title">
  <h2 id="action-title">{t(`link.${link.action}`, { name: target })}</h2>
  <p>{t('link.source', { collection: link.collection })} · <code>{link.kind}/{link.id}</code></p>
  {#if store.loading}
    <p role="status">{t('catalog.loading')}</p>
  {:else if store.catalogError || error}
    <p role="alert">{error || t('toast.loadCollections')}</p>
  {:else if missingCollection || !collection}
    <p role="alert">{t('link.missingCollection', { collection: link.collection })}</p>
  {:else if !app && !kit}
    <p role="alert">{t('deps.notFound')}</p>
  {:else if failed}
    <p role="alert">{failed} · {store.statusErrors[failed] ? terror(store.statusErrors[failed]) : t('deps.notFound')}</p>
  {:else if checking}
    <p role="status">{t('app.status.checking')}</p>
  {:else if !apps.length}
    <p>{t('link.noAction')}</p>
  {:else}
    {#if kit}
      <ul>{#each apps as member (member.key)}<li>{appName(member)} <code>{member.key}</code></li>{/each}</ul>
    {/if}
    {#if link.action !== 'uninstall' && apps.some(a => a.dependencies.length)}
      <p>{t('link.dependencies')}</p>
    {/if}
    {#if link.action === 'uninstall'}<RemovalImpact keys={apps.map(a => a.key)} removingKit={kit?.key} />{/if}
  {/if}
  <div class="actions">
    <button class="secondary" onclick={() => dismissLink(url)}>{t('common.cancel')}</button>
    {#if !store.loading && !store.catalogError && (missingCollection || !collection) && store.usersCanAddCollections}
      <button class="secondary" onclick={collections}>{t('catalog.addCollection')}</button>
    {/if}
    {#if ready}<button class={link.action === 'uninstall' ? 'danger' : 'primary'} disabled={submitting}
      onclick={confirm}>{needsAnswers ? t('action.answerFirst') : t(`common.${link.action}`)}</button>{/if}
  </div>
</dialog>

<style>
  dialog { width: min(30rem, calc(100vw - 2rem)); max-height: 85vh; overflow: auto; overflow-wrap: anywhere;
    padding: 1.5rem; border: 1px solid var(--line); border-radius: var(--radius);
    background: var(--surface); color: var(--ink); box-shadow: var(--shadow-raised); }
  dialog::backdrop { background: rgb(10 25 28 / .4); }
  h2 { margin: 0; font: 700 1.35rem var(--font-display); }
  p, li { color: var(--ink-soft); }
  code { font: .85em var(--font-mono); }
  .actions { display: flex; justify-content: flex-end; gap: .5rem; margin-top: 1.25rem; }
</style>

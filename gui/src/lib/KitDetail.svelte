<script lang="ts">
  import Description from './Description.svelte';
  import ActionLinks from './ActionLinks.svelte';
  import CollectionAvatar from './CollectionAvatar.svelte';
  // A kit: choose which of its apps to install (required ones can't be deselected), or which
  // installed ones to uninstall (preselected: those no other installed kit needs).
  import type { KitInfo } from './api';
  import {
    appStatus,
    store,
    appByKey,
    isInstalled,
    isPending,
    pendingUninstall,
    installKit,
    uninstallKit,
    defaultUninstall,
    otherKitsWith,
    appName,
    appDescription,
    kitName,
    kitStatus,
    kitDescription,
    categoryName,
    externalStatus, packageUpdateLabel,
    requiredUpdate,
  } from './state.svelte';
  import RemovalImpact from './RemovalImpact.svelte';
  import AppIcon from './AppIcon.svelte';
  import { t } from './i18n.svelte';

  let { kit, onback, onopen }: { kit: KitInfo; onback: () => void; onopen: (key: string) => void } = $props();

  // Install choices for optional items the user changed; everything else follows the kit's defaults.
  let picked = $state<Record<string, boolean>>({});
  let removing = $state(false);
  let removePicked = $state<Record<string, boolean>>({});

  const status = $derived(kitStatus(kit));
  const unavailable = (key: string) => appStatus(key) === 'unavailable';
  /** Items the install checklist can't change: required, installed, running/queued or not for this OS. */
  const locked = (item: KitInfo['items'][number]) =>
    item.required || isInstalled(item.key) || isPending(item.key) || unavailable(item.key);
  const checked = (item: KitInfo['items'][number]) =>
    unavailable(item.key) || (!item.required && pendingUninstall(item.key)) ? false : locked(item) ? true : (picked[item.key] ?? item.selected);

  const toInstall = $derived(
    kit.items.filter((i) => checked(i) && !isInstalled(i.key) && !isPending(i.key) && !unavailable(i.key)).map((i) => i.key),
  );
  const conflict = $derived(kit.items.find(i => i.required && !unavailable(i.key) && pendingUninstall(i.key)));
  const installedItems = $derived(kit.items.filter((i) => isInstalled(i.key)));
  const defaults = $derived(defaultUninstall(kit));
  const toRemove = $derived(
    installedItems.filter((i) => store.checks[i.key]?.removable !== false && (removePicked[i.key] ?? defaults.includes(i.key))).map((i) => i.key),
  );

  function badge(item: KitInfo['items'][number]) {
    const key = item.key;
    if (store.busy[key]) return store.busy[key] === 'uninstall' ? t('common.uninstalling') : t('common.installing');
    if (store.queue.some((q) => q.key === key)) return t('common.queued');
    if (unavailable(key)) return t('common.notForThisSystem');
    if (requiredUpdate(key)) return t('common.securityUpdate');
    if (appStatus(key) === 'needsUpdate') return t('common.updateAvailable');
    if (isInstalled(key)) return packageUpdateLabel(key) ?? t('common.installed');
    if (item.required) return t('common.required');
    return '';
  }

  function install() {
    const selected = kit.items.filter(checked);
    installKit(kit, selected.map(item => item.key));
    picked = {};
  }
  function uninstall() {
    const queued = uninstallKit(kit, toRemove);
    removing = false;
    removePicked = {};
    return queued;
  }
</script>

<section class="detail">
  <div class="toolbar">
    <button class="back" onclick={onback}>{t('common.back')}</button>
    <ActionLinks kind="kit" collection={kit.collection} id={kit.id} name={kitName(kit)} />
  </div>

  <header>
    <AppIcon app={kit} name={kitName(kit)} size={96} />
    <div class="heading">
      <p class="eyebrow">{t('kit.eyebrow', { apps: t('common.apps', { n: kit.items.length }) })}</p>
      <h1>{kitName(kit)}</h1>
      <p class="meta">{kit.categories.map((c) => categoryName(c, kit.collection)).join(' · ')}{kit.categories.length ? t('common.metaSeparator') : ''}{t('common.from')} <CollectionAvatar name={kit.collection} size={20} /> {kit.collection}</p>
    </div>
  </header>
  {#if kitDescription(kit)}<div class="description"><Description text={kitDescription(kit)} collection={kit.collection} /></div>{/if}

  {#if !removing}
    <h2>{t('kit.chooseInstall')}</h2>
    <p class="meta">{t('kit.partlyInstalled', { n: status.installed, total: status.applicable })}
      {#if status.unavailable} · {t('kit.otherSystems', { n: status.unavailable })}{/if}
    </p>
    <ul class="items">
      {#each kit.items as item (item.key)}
        {@const app = appByKey(item.key)}
        {#if app}
          <li class:muted={unavailable(item.key)}>
            <label>
              <input
                type="checkbox"
                checked={checked(item)}
                disabled={locked(item)}
                onchange={(e) => (picked[item.key] = e.currentTarget.checked)}
              />
              <AppIcon {app} size={36} />
              <span class="text">
                <span><span class="name">{appName(app)}</span>{#if !item.required} <small>· {t('common.optional')}</small>{/if}</span>
                <span class="desc"><Description text={appDescription(app)} collection={app.collection} compact /></span>
              </span>
            </label>
            {#if badge(item)}<span class="badge" class:required={badge(item) === t('common.required')}>{badge(item)}</span>{/if}
            <button class="link" onclick={() => onopen(item.key)} aria-label={t('kit.detailsOf', { app: appName(app) })}>{t('kit.details')}</button>
          </li>
        {/if}
      {/each}
    </ul>
    <div class="actions">
      <button class="primary big" disabled={!toInstall.length || !!conflict} onclick={install}>
        {toInstall.length ? t('kit.install', { n: toInstall.length }) : t('kit.nothingToInstall')}
      </button>
      {#if installedItems.length}
        <button class="secondary big" onclick={() => (removing = true)}>{t('kit.uninstallApps')}</button>
      {/if}
    </div>
    {#if conflict}
      {@const app = appByKey(conflict.key)}
      <p class="hint" role="status">{t('queue.uninstalling', { detail: app ? appName(app) : conflict.key })}</p>
    {:else}
      <p class="hint">{t('kit.hint')}</p>
    {/if}
  {:else}
    <h2>{t('kit.chooseUninstall')}</h2>
    <ul class="items">
      {#each installedItems as item (item.key)}
        {@const check = store.checks[item.key]}
        {@const app = appByKey(item.key)}
        {@const others = otherKitsWith(item.key, kit)}
        {#if app}
          <li>
            <label>
              <input
                type="checkbox"
                checked={toRemove.includes(item.key)}
                disabled={isPending(item.key) || check?.removable === false}
                onchange={(e) => (removePicked[item.key] = e.currentTarget.checked)}
              />
              <AppIcon {app} size={36} />
              <span class="text">
                <span class="name">{appName(app)}</span>
                {#if packageUpdateLabel(item.key)}<span class="desc">{packageUpdateLabel(item.key)}</span>{/if}
                {#if check?.external}
                  <span class="desc">{externalStatus(check?.external)}</span>
                {:else if check?.removable === false}
                  <span class="desc">{t('kit.cantUninstall')}</span>
                {:else if others.length}
                  <span class="desc">{t('kit.alsoIn', { kits: others.map(kitName).join(', ') })}</span>
                {:else}
                  <span class="desc"><Description text={appDescription(app)} collection={app.collection} compact /></span>
                {/if}
              </span>
            </label>
          </li>
        {/if}
      {/each}
    </ul>
    <RemovalImpact keys={toRemove} removingKit={kit.key} />
    <div class="actions">
      <button class="danger big" disabled={!toRemove.length} onclick={uninstall}>
        {toRemove.length ? t('kit.uninstall', { n: toRemove.length }) : t('kit.nothingSelected')}
      </button>
      <button class="secondary big" onclick={() => ((removing = false), (removePicked = {}))}>{t('common.cancel')}</button>
    </div>
  {/if}
</section>

<style>
  .text small { font-weight: 400; color: var(--muted); }
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
  }
  .eyebrow {
    margin: 0;
    font: 700 0.72rem var(--font-body);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--accent-ink);
  }
  h1 {
    margin: 0.15rem 0 0;
    font: 700 2.4rem/1.05 var(--font-display);
    letter-spacing: -0.02em;
  }
  .meta {
    margin: 0.4rem 0 0;
    color: var(--muted);
  }
  .description {
    margin: 1.5rem 0 0;
    font-size: 1.05rem;
    line-height: 1.6;
    color: var(--ink-soft);
  }
  h2 {
    margin: 2rem 0 0.75rem;
    font: 650 1rem var(--font-display);
  }
  .items {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  .items li {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 0;
    border-bottom: 1px solid var(--line);
  }
  .items li.muted {
    opacity: 0.55;
  }
  label {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.85rem;
    cursor: pointer;
  }
  input {
    flex: none;
    width: 1.15rem;
    height: 1.15rem;
    accent-color: var(--accent);
  }
  input:disabled + :global(*) {
    cursor: default;
  }
  .text {
    display: grid;
    min-width: 0;
  }
  .name {
    font-weight: 650;
  }
  .desc {
    font-size: 0.85rem;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    flex: none;
    padding: 0.15rem 0.6rem;
    border-radius: 999px;
    font-size: 0.75rem;
    font-weight: 650;
    color: var(--ink-soft);
    background: var(--hover);
  }
  .badge.required {
    color: var(--accent-ink);
    background: var(--accent-soft);
  }
  .link {
    flex: none;
    border: 0;
    background: none;
    color: var(--accent-ink);
    font: 600 0.85rem var(--font-body);
    cursor: pointer;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    margin-top: 1.25rem;
  }
  .big {
    height: 2.75rem;
    padding: 0 1.4rem;
    border-radius: 999px;
    font-size: 0.95rem;
  }
  .hint {
    font-size: 0.82rem;
    color: var(--muted);
  }
</style>

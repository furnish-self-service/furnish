<script lang="ts">
  // A kit: choose which of its apps to install (required ones can't be deselected), or which
  // installed ones to uninstall (preselected: those no other installed kit needs).
  import type { KitInfo } from './api';
  import {
    store,
    appByKey,
    isInstalled,
    isPending,
    installKit,
    uninstallKit,
    defaultUninstall,
    otherKitsWith,
  } from './state.svelte';
  import AppIcon from './AppIcon.svelte';

  let { kit, onback, onopen }: { kit: KitInfo; onback: () => void; onopen: (key: string) => void } = $props();

  // Install choices for optional items the user changed; everything else follows the kit's defaults.
  let picked = $state<Record<string, boolean>>({});
  let removing = $state(false);
  let removePicked = $state<Record<string, boolean>>({});

  const unavailable = (key: string) => store.status[key] === 'unavailable';
  /** Items the install checklist can't change: required, installed, running/queued or not for this OS. */
  const locked = (item: KitInfo['items'][number]) =>
    item.required || isInstalled(item.key) || isPending(item.key) || unavailable(item.key);
  const checked = (item: KitInfo['items'][number]) =>
    unavailable(item.key) ? false : locked(item) ? true : (picked[item.key] ?? item.selected);

  const toInstall = $derived(
    kit.items.filter((i) => checked(i) && !isInstalled(i.key) && !isPending(i.key) && !unavailable(i.key)).map((i) => i.key),
  );
  const installedItems = $derived(kit.items.filter((i) => isInstalled(i.key)));
  const defaults = $derived(defaultUninstall(kit));
  const toRemove = $derived(installedItems.filter((i) => removePicked[i.key] ?? defaults.includes(i.key)).map((i) => i.key));
  const count = (n: number) => `${n} app${n === 1 ? '' : 's'}`;

  function badge(item: KitInfo['items'][number]) {
    const key = item.key;
    if (store.busy[key]) return store.busy[key] === 'uninstall' ? 'Uninstalling…' : 'Installing…';
    if (store.queue.some((q) => q.key === key)) return 'Queued';
    if (unavailable(key)) return 'Not for this system';
    if (store.status[key] === 'needsUpdate') return 'Update available';
    if (isInstalled(key)) return 'Installed';
    if (item.required) return 'Required';
    return '';
  }

  function install() {
    installKit(kit, toInstall);
    picked = {};
  }
  function uninstall() {
    uninstallKit(kit, toRemove);
    removing = false;
    removePicked = {};
  }
</script>

<section class="detail">
  <button class="back" onclick={onback}>← Back</button>

  <header>
    <AppIcon app={kit} size={96} />
    <div class="heading">
      <p class="eyebrow">Kit · {count(kit.items.length)}</p>
      <h1>{kit.id}</h1>
      <p class="meta">{kit.categories.join(' · ')}{kit.categories.length ? ' — ' : ''}from {kit.collection}</p>
    </div>
  </header>
  {#if kit.description}<p class="description">{kit.description}</p>{/if}

  {#if !removing}
    <h2>Choose apps to install</h2>
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
                <span class="name">{app.id}</span>
                <span class="desc">{app.description}</span>
              </span>
            </label>
            {#if badge(item)}<span class="badge" class:required={badge(item) === 'Required'}>{badge(item)}</span>{/if}
            <button class="link" onclick={() => onopen(item.key)} aria-label="Details of {app.id}">Details</button>
          </li>
        {/if}
      {/each}
    </ul>
    <div class="actions">
      <button class="primary big" disabled={!toInstall.length} onclick={install}>
        {toInstall.length ? `Install ${count(toInstall.length)}` : 'Nothing to install'}
      </button>
      {#if installedItems.length}
        <button class="secondary big" onclick={() => (removing = true)}>Uninstall apps…</button>
      {/if}
    </div>
    <p class="hint">Installs run one after another; follow them under Activity.</p>
  {:else}
    <h2>Choose apps to uninstall</h2>
    <ul class="items">
      {#each installedItems as item (item.key)}
        {@const app = appByKey(item.key)}
        {@const others = otherKitsWith(item.key, kit)}
        {#if app}
          <li>
            <label>
              <input
                type="checkbox"
                checked={toRemove.includes(item.key)}
                disabled={isPending(item.key)}
                onchange={(e) => (removePicked[item.key] = e.currentTarget.checked)}
              />
              <AppIcon {app} size={36} />
              <span class="text">
                <span class="name">{app.id}</span>
                {#if others.length}
                  <span class="desc">Also in {others.map((k) => k.id).join(', ')}, which is installed</span>
                {:else}
                  <span class="desc">{app.description}</span>
                {/if}
              </span>
            </label>
          </li>
        {/if}
      {/each}
    </ul>
    <div class="actions">
      <button class="danger big" disabled={!toRemove.length} onclick={uninstall}>
        {toRemove.length ? `Uninstall ${count(toRemove.length)}` : 'Nothing selected'}
      </button>
      <button class="secondary big" onclick={() => ((removing = false), (removePicked = {}))}>Cancel</button>
    </div>
  {/if}
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

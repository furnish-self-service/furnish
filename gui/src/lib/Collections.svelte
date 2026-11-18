<script lang="ts">
  import * as api from './api';
  import Options from './Options.svelte';
  import CollectionTracking from './CollectionTracking.svelte';
  import TrackingFields from './TrackingFields.svelte';
  import CollectionSettings from './CollectionSettings.svelte';
  import { askQuestions, refreshEndpoints } from './state.svelte';
  import { store, collectionAction, updateCollections, toast } from './state.svelte';
  import { t } from './i18n.svelte';

  let url = $state('');
  let name = $state('');
  let tracking = $state<api.Tracking | null>(null);
  let showTracking = $state(false);
  let target = $state<{ input: string; endpoint: boolean }>();
  const address = $derived(url.trim());
  const repository = $derived(target?.input === address && !target.endpoint);
  $effect(() => {
    const input = address;
    if (!input) return;
    let current = true;
    api.isFurnishAddress(input).then(endpoint => {
      if (current) target = { input, endpoint };
    }).catch(() => {
      // The add command validates the target again; preserve the original add flow.
      if (current) target = { input, endpoint: false };
    });
    return () => { current = false; };
  });
  let adding = $state(false);
  // HTTPS repositories that need a login: username and password (or access token).
  let username = $state('');
  let password = $state('');
  let needsLogin = $state(false);
  const https = $derived(/^https?:\/\/.+\/./.test(url.trim()));
  /** Name of the collection being updated or removed; all-update uses '*'. */
  let working = $state<string | null>(null);
  let confirmRemove = $state<string | null>(null);

  async function add(event: SubmitEvent) {
    event.preventDefault();
    if (adding || !address || target?.input !== address) return;
    adding = true;
    const ok = await collectionAction(async () => {
      try {
        const added = await api.addCollection(url, name || null, username, password, repository ? tracking : null);
        if (!added.length) return t('collections.signedInAlready');
        return t('collections.added', { names: added.map((c) => c.name).join(', ') });
      } catch (e) {
        // The server wants a login: show the fields (furnish keeps it in the credential store).
        if (String(e).includes('needs a username and password')) needsLogin = true;
        throw e;
      }
    }, t('collections.addFailed'));
    adding = false;
    store.signInUrl = null;
    await refreshEndpoints();
    if (ok) {
      url = name = username = password = '';
      needsLogin = false;
      tracking = null;
      showTracking = false;
      askQuestions(true); // the new collection's required options
    }
  }

  async function update(target: string) {
    working = target;
    if (target === '*') {
      try {
        const changed = await updateCollections();
        toast('success', changed.length ? t('collections.updated', { names: changed.join(', ') }) : t('collections.allUpToDate'));
      } catch (e) {
        toast('error', t('collections.updateAllFailed'), e instanceof Error ? e.message : String(e));
      }
    } else {
      await collectionAction(async () => {
        const r = await api.updateCollection(target);
        return r.old !== r.new ? t('collections.updated', { names: target }) : t('collections.isUpToDate', { name: target });
      }, t('collections.updateFailed'));
    }
    working = null;
  }

  async function remove(target: string) {
    working = target;
    confirmRemove = null;
    await collectionAction(async () => {
      await api.removeCollection(target);
      return t('collections.removed', { name: target });
    }, t('collections.removeFailed', { name: target }));
    working = null;
  }
</script>

<section>
  <h1>{t('collections.title')}</h1>
  <p class="lead">{t('collections.lead')}</p>

  {#if store.usersCanAddCollections}
  <form onsubmit={add}>
    <label class="url">
      <span>{t('collections.url')}</span>
      <input bind:value={url} required placeholder="git@github.com:your-org/apps.git" spellcheck="false" />
    </label>
    <label class="name">
      <span>{t('collections.name')} <em>{t('collections.optional')}</em></span>
      <input bind:value={name} placeholder={t('collections.namePlaceholder')} spellcheck="false" />
    </label>
    <button class="primary" disabled={adding || !address || target?.input !== address}>{adding ? t('collections.adding') : t('collections.add')}</button>
    {#if repository}
      <details class="login" bind:open={showTracking}>
        <summary>{t('tracking.heading')}</summary>
        <TrackingFields bind:value={tracking} disabled={adding} />
      </details>
    {/if}
    {#if https && repository}
      <details class="login" bind:open={needsLogin}>
        <summary>{t('collections.needsLogin')}</summary>
        <p class="explain flush">{t('collections.loginText')}</p>
        <label class="name">
          <span>{t('collections.username')}</span>
          <input bind:value={username} autocomplete="username" spellcheck="false" />
        </label>
        <label class="name">
          <span>{t('collections.password')}</span>
          <input bind:value={password} type="password" autocomplete="current-password" />
        </label>
      </details>
    {/if}
  </form>
  <p class="warning">{t('collections.warning')}</p>
  {:else}
    <p class="managed-note">{t('collections.managedNote')}</p>
  {/if}

  {#if store.collections.length}
    <div class="toolbar">
      <h2>{t('collections.count', { n: store.collections.length })}</h2>
      <button class="secondary" disabled={working !== null} onclick={() => update('*')}>
        {working === '*' ? t('collections.updating') : t('common.updateAll')}
      </button>
    </div>
    <ul>
      {#each store.collections as c (c.name)}
        <li>
          {#if c.icon}
            <picture>
              {#if c.iconDark}<source media="(prefers-color-scheme: dark)" srcset={c.iconDark} />{/if}
              <img class="avatar" src={c.icon} alt="" width="36" height="36" />
            </picture>
          {/if}
          <div class="info">
            <span class="cname">{c.name}</span>
            <span class="mono">{c.url}</span>
          </div>
          <span class="mono rev" title={t('collections.revision')}>{c.revision}</span>
          <span class="count">{t('common.apps', { n: c.apps })}</span>
          {#if c.updateAvailable}<span class="available">{t('common.updateAvailable')}</span>{/if}
          {#if c.managed}
            <span class="actions">
              <span class="org">{t('collections.managed')}</span>
              <button class="secondary" disabled={working !== null} onclick={() => update(c.name)}>
                {working === c.name ? t('collections.updating') : t('common.update')}
              </button>
            </span>
          {:else if confirmRemove === c.name}
            <span class="confirm">
              {t('collections.confirmRemove', { name: c.name })}
              <button class="danger" onclick={() => remove(c.name)}>{t('common.remove')}</button>
              <button class="secondary" onclick={() => (confirmRemove = null)}>{t('common.cancel')}</button>
            </span>
          {:else}
            <span class="actions">
              <button class="secondary" disabled={working !== null} onclick={() => update(c.name)}>
                {working === c.name ? t('collections.updating') : t('common.update')}
              </button>
              <button class="secondary" disabled={working !== null} onclick={() => (confirmRemove = c.name)}>{t('common.remove')}</button>
            </span>
          {/if}
          {#if c.options.length}
            <details class="copts">
              <summary>
                {t('collections.optionsFor', { name: c.name })}
                {#if c.options.some((o) => o.missing)}<span class="org">{t('collections.needsAnswer')}</span>{/if}
              </summary>
              <Options target={c.name} options={c.options} scope="collection" />
            </details>
          {/if}
          <details class="copts" data-tracking={c.name}>
            <summary>{t('tracking.heading')}</summary>
            <CollectionTracking collection={c} />
          </details>
          <details class="copts">
            <summary>{t('collections.downloads')}</summary>
            <CollectionSettings collection={c} />
          </details>
        </li>
      {/each}
    </ul>
  {/if}

</section>

<style>
  section {
    max-width: 56rem;
  }
  h1 {
    margin: 0;
    font: 700 2rem/1.1 var(--font-display);
    letter-spacing: -0.02em;
  }
  .lead {
    color: var(--ink-soft);
    max-width: 38rem;
    line-height: 1.5;
  }
  form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    align-items: end;
    margin-top: 1.5rem;
  }
  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--ink-soft);
  }
  label em {
    font-weight: 400;
    color: var(--muted);
  }
  .url {
    flex: 3 1 18rem;
  }
  .name {
    flex: 1 1 9rem;
  }
  .warning {
    font-size: 0.8rem;
    color: var(--muted);
  }
  .org {
    margin-left: 0.4rem;
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 650;
    color: var(--ink-soft);
    background: var(--hover);
    white-space: nowrap;
  }
  .managed-note {
    margin-top: 1.5rem;
    color: var(--ink-soft);
  }
  .explain {
    margin: 0;
    font-size: 0.8rem;
    color: var(--muted);
  }
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 2.5rem;
  }
  h2 {
    margin: 0;
    font: 650 1rem var(--font-display);
  }
  ul {
    margin: 0.75rem 0 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 1.25rem;
    padding: 0.85rem 0;
    border-bottom: 1px solid var(--line);
  }
  .copts,
  .login {
    flex-basis: 100%;
  }
  .login summary {
    cursor: pointer;
    font-size: 0.88rem;
    font-weight: 600;
    color: var(--ink-soft);
  }
  .login[open] {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
  }
  .login[open] summary {
    flex-basis: 100%;
  }
  .avatar {
    border-radius: 9px;
  }
  .copts summary {
    cursor: pointer;
    font-size: 0.88rem;
    font-weight: 600;
    color: var(--ink-soft);
  }
  .info {
    display: grid;
    gap: 0.2rem;
    flex: 1 1 16rem;
    min-width: 0;
  }
  .cname {
    font-weight: 650;
  }
  .mono {
    font: 0.78rem var(--font-mono);
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .available {
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 700;
    background: var(--highlight-soft);
    color: var(--highlight-ink);
    white-space: nowrap;
  }
  .count {
    font-size: 0.85rem;
    color: var(--ink-soft);
  }
  .actions,
  .confirm {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    font-size: 0.85rem;
  }
</style>

<script lang="ts">
  import * as api from './api';
  import Options from './Options.svelte';
  import CollectionSettings from './CollectionSettings.svelte';
  import { askQuestions, refreshEndpoints } from './state.svelte';
  import { store, collectionAction, changeSettings, updateCollections, toast, isLocked } from './state.svelte';

  const intervals = [
    [15, 'Every 15 minutes'],
    [30, 'Every 30 minutes'],
    [60, 'Every hour'],
    [240, 'Every 4 hours'],
    [1440, 'Every day'],
    [0, 'Never'],
  ] as const;
  // A value set outside the app (settings.json) still shows up as a choice.
  const options = $derived(
    store.settings && !intervals.some(([m]) => m === store.settings!.updateIntervalMinutes)
      ? [...intervals, [store.settings.updateIntervalMinutes, `Every ${store.settings.updateIntervalMinutes} minutes`] as const]
      : intervals,
  );
  const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

  let url = $state('');
  let name = $state('');
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
    adding = true;
    const ok = await collectionAction(async () => {
      try {
        const added = await api.addCollection(url, name || null, username, password);
        if (!added.length) return 'Signed in; its collections were already there';
        return `Added ${added.map((c) => c.name).join(', ')}`;
      } catch (e) {
        // The server wants a login: show the fields (furnish keeps it in the credential store).
        if (String(e).includes('needs a username and password')) needsLogin = true;
        throw e;
      }
    }, "Couldn't add the collection");
    adding = false;
    store.signInUrl = null;
    await refreshEndpoints();
    if (ok) {
      url = name = username = password = '';
      needsLogin = false;
      askQuestions(true); // the new collection's required options
    }
  }

  async function update(target: string) {
    working = target;
    if (target === '*') {
      try {
        const changed = await updateCollections();
        toast('success', changed.length ? `Updated ${changed.join(', ')}` : 'Everything is up to date');
      } catch (e) {
        toast('error', "Couldn't update every collection", e instanceof Error ? e.message : String(e));
      }
    } else {
      await collectionAction(async () => {
        const r = await api.updateCollection(target);
        return r.old !== r.new ? `Updated ${target}` : `${target} is up to date`;
      }, "Couldn't update");
    }
    working = null;
  }

  async function remove(target: string) {
    working = target;
    confirmRemove = null;
    await collectionAction(async () => {
      await api.removeCollection(target);
      return `Removed ${target}`;
    }, `Couldn't remove ${target}`);
    working = null;
  }
</script>

<section>
  <h1>Collections</h1>
  <p class="lead">
    A collection is a git repository with app definitions. Apps you installed stay installed when you
    remove their collection.
  </p>

  {#if store.usersCanAddCollections}
  <form onsubmit={add}>
    <label class="url">
      <span>Git URL, or your organization's furnish address</span>
      <input bind:value={url} required placeholder="git@github.com:your-org/apps.git" spellcheck="false" />
    </label>
    <label class="name">
      <span>Name <em>(optional)</em></span>
      <input bind:value={name} placeholder="from the URL" spellcheck="false" />
    </label>
    <button class="primary" disabled={adding || !url.trim()}>{adding ? 'Adding…' : 'Add collection'}</button>
    {#if https}
      <details class="login" bind:open={needsLogin}>
        <summary>Needs a login?</summary>
        <p class="explain flush">For a private repository: your username and a password or access token. furnish keeps them in your system's credential store and sends them to this repository only. SSH URLs use your SSH keys and agent instead.</p>
        <label class="name">
          <span>Username</span>
          <input bind:value={username} autocomplete="username" spellcheck="false" />
        </label>
        <label class="name">
          <span>Password or access token</span>
          <input bind:value={password} type="password" autocomplete="current-password" />
        </label>
      </details>
    {/if}
  </form>
  <p class="warning">Apps run with administrator rights. Only add collections from people you trust.</p>
  {:else}
    <p class="managed-note">Your organization manages the collections on this computer.</p>
  {/if}

  {#if store.settings}
    <section class="auto" aria-labelledby="auto-title">
      <h2 id="auto-title">Automatic updates</h2>
      <p class="explain flush">Installed apps update automatically; turn that off for an app on its page.</p>
      <label class="check">
        <input
          type="checkbox"
          checked={store.settings.updateOnLaunch}
          disabled={isLocked('updateOnLaunch')}
          onchange={(e) => changeSettings({ updateOnLaunch: e.currentTarget.checked })}
        />
        Update collections when {store.brand.name} starts
        {#if isLocked('updateOnLaunch')}<span class="org">Set by your organization</span>{/if}
      </label>
      <label class="interval">
        <span>While {store.brand.name} is open</span>
        <select
          value={store.settings.updateIntervalMinutes}
          disabled={isLocked('updateIntervalMinutes')}
          onchange={(e) => changeSettings({ updateIntervalMinutes: Number(e.currentTarget.value) })}
        >
          {#each options as [minutes, text]}
            <option value={minutes}>{minutes === 0 ? "Don't check" : text}</option>
          {/each}
        </select>
        {#if isLocked('updateIntervalMinutes')}<span class="org">Set by your organization</span>{/if}
      </label>
      <label class="check">
        <input
          type="checkbox"
          checked={store.settings.loadAwareUpdates}
          disabled={isLocked('loadAwareUpdates')}
          onchange={(e) => changeSettings({ loadAwareUpdates: e.currentTarget.checked })}
        />
        Postpone automatic updates while this computer is busy
        {#if isLocked('loadAwareUpdates')}<span class="org">Set by your organization</span>{/if}
      </label>
      <p class="explain">
        Busy means the last 5 minutes averaged over {store.settings.loadLimits.cpuPercent}% CPU,
        {store.settings.loadLimits.memoryPercent}% memory or {store.settings.loadLimits.networkKbps} KB/s of network
        traffic, not counting {store.brand.name} itself.
      </p>
      <p class="last" aria-live="polite">
        {#if store.autoUpdate.running}
          Checking for updates…
        {:else if store.autoUpdate.error}
          <span class="failed">Last automatic check failed:</span> {store.autoUpdate.error}
        {:else if store.autoUpdate.lastAt}
          Last checked at {clock(store.autoUpdate.lastAt)}.
        {/if}
      </p>
    </section>
  {/if}

  {#if store.collections.length}
    <div class="toolbar">
      <h2>{store.collections.length} collection{store.collections.length === 1 ? '' : 's'}</h2>
      <button class="secondary" disabled={working !== null} onclick={() => update('*')}>
        {working === '*' ? 'Updating…' : 'Update all'}
      </button>
    </div>
    <ul>
      {#each store.collections as c (c.name)}
        <li>
          {#if c.icon}<img class="avatar" src={c.icon} alt="" width="36" height="36" />{/if}
          <div class="info">
            <span class="cname">{c.name}</span>
            <span class="mono">{c.url}</span>
          </div>
          <span class="mono rev" title="Revision">{c.revision}</span>
          <span class="count">{c.apps} app{c.apps === 1 ? '' : 's'}</span>
          {#if c.managed}
            <span class="actions">
              <span class="org">Managed by your organization</span>
              <button class="secondary" disabled={working !== null} onclick={() => update(c.name)}>
                {working === c.name ? 'Updating…' : 'Update'}
              </button>
            </span>
          {:else if confirmRemove === c.name}
            <span class="confirm">
              Remove {c.name}?
              <button class="danger" onclick={() => remove(c.name)}>Remove</button>
              <button class="secondary" onclick={() => (confirmRemove = null)}>Cancel</button>
            </span>
          {:else}
            <span class="actions">
              <button class="secondary" disabled={working !== null} onclick={() => update(c.name)}>
                {working === c.name ? 'Updating…' : 'Update'}
              </button>
              <button class="secondary" disabled={working !== null} onclick={() => (confirmRemove = c.name)}>Remove</button>
            </span>
          {/if}
          {#if c.options.length}
            <details class="copts">
              <summary>
                Options for all {c.name} apps
                {#if c.options.some((o) => o.missing)}<span class="org">Needs your answer</span>{/if}
              </summary>
              <Options target={c.name} options={c.options} scope="collection" />
            </details>
          {/if}
          <details class="copts">
            <summary>Downloads and earlier versions</summary>
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
  .auto {
    display: grid;
    gap: 0.6rem;
    margin-top: 2.5rem;
    max-width: 36rem;
  }
  .check {
    display: flex;
    gap: 0.55rem;
    align-items: center;
    font-size: 0.92rem;
    font-weight: 400;
    color: var(--ink);
  }
  .check input {
    width: 1.05rem;
    height: 1.05rem;
    accent-color: var(--accent);
  }
  .interval {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    font-size: 0.92rem;
    font-weight: 400;
    color: var(--ink);
  }
  select {
    padding: 0.45rem 0.7rem;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: 0.92rem var(--font-body);
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
  .explain.flush {
    margin-left: 0;
  }
  .explain {
    margin: -0.3rem 0 0 1.6rem;
    font-size: 0.8rem;
    color: var(--muted);
  }
  .last {
    margin: 0;
    font-size: 0.82rem;
    color: var(--muted);
    white-space: pre-line;
  }
  .failed {
    color: var(--danger);
    font-weight: 600;
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

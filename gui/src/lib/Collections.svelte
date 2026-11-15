<script lang="ts">
  import * as api from './api';
  import Options from './Options.svelte';
  import CollectionSettings from './CollectionSettings.svelte';
  import { askQuestions, refreshEndpoints } from './state.svelte';
  import { store, collectionAction, changeSettings, updateCollections, toast, isLocked } from './state.svelte';
  import { t, parts, formatTime } from './i18n.svelte';
  import { languages, type Lang } from './i18n.ts';

  const intervals = $derived([
    [15, t('collections.every15')],
    [30, t('collections.every30')],
    [60, t('collections.everyHour')],
    [240, t('collections.every4Hours')],
    [1440, t('collections.everyDay')],
    [0, t('collections.dontCheck')],
  ] as [number, string][]);
  // A value set outside the app (settings.json) still shows up as a choice.
  const options = $derived(
    store.settings && !intervals.some(([m]) => m === store.settings!.updateIntervalMinutes)
      ? [...intervals, [store.settings.updateIntervalMinutes, t('collections.everyMinutes', { n: store.settings.updateIntervalMinutes })]]
      : intervals,
  );
  const clock = (ms: number) => formatTime(ms);

  let version = $state('');
  api.version().then((v) => (version = v));
  // The organization's furnish addresses that take reports; the section shows only with one.
  let destinations = $state<string[]>([]);
  api.reportDestinations().then((d) => (destinations = d));

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
    <button class="primary" disabled={adding || !url.trim()}>{adding ? t('collections.adding') : t('collections.add')}</button>
    {#if https}
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

  {#if store.settings}
    <section class="auto" aria-labelledby="auto-title">
      <h2 id="auto-title">{t('collections.autoUpdates')}</h2>
      <p class="explain flush">{t('collections.autoUpdatesText')}</p>
      <label class="check">
        <input
          type="checkbox"
          checked={store.settings.updateOnLaunch}
          disabled={isLocked('updateOnLaunch')}
          onchange={(e) => changeSettings({ updateOnLaunch: e.currentTarget.checked })}
        />
        {t('collections.updateOnLaunch', { brand: store.brand.name })}
        {#if isLocked('updateOnLaunch')}<span class="org">{t('common.setByOrg')}</span>{/if}
      </label>
      <label class="interval">
        <span>{t('collections.whileOpen', { brand: store.brand.name })}</span>
        <select
          value={store.settings.updateIntervalMinutes}
          disabled={isLocked('updateIntervalMinutes')}
          onchange={(e) => changeSettings({ updateIntervalMinutes: Number(e.currentTarget.value) })}
        >
          {#each options as [minutes, text]}
            <option value={minutes}>{text}</option>
          {/each}
        </select>
        {#if isLocked('updateIntervalMinutes')}<span class="org">{t('common.setByOrg')}</span>{/if}
      </label>
      <label class="check">
        <input
          type="checkbox"
          checked={store.settings.loadAwareUpdates}
          disabled={isLocked('loadAwareUpdates')}
          onchange={(e) => changeSettings({ loadAwareUpdates: e.currentTarget.checked })}
        />
        {t('collections.loadAware')}
        {#if isLocked('loadAwareUpdates')}<span class="org">{t('common.setByOrg')}</span>{/if}
      </label>
      <p class="explain">
        {t('collections.busyMeans', {
          cpu: store.settings.loadLimits.cpuPercent,
          memory: store.settings.loadLimits.memoryPercent,
          network: store.settings.loadLimits.networkKbps,
          brand: store.brand.name,
        })}
      </p>
      <p class="last" aria-live="polite">
        {#if store.autoUpdate.running}
          {t('collections.checking')}
        {:else if store.autoUpdate.error}
          <span class="failed">{t('collections.lastFailed')}</span> {store.autoUpdate.error}
        {:else if store.autoUpdate.lastAt}
          {t('collections.lastChecked', { time: clock(store.autoUpdate.lastAt) })}
        {/if}
      </p>
    </section>
  {/if}

  {#if store.settings && (destinations.length || isLocked('reporting'))}
    <section class="auto" aria-labelledby="reports-title">
      <h2 id="reports-title">{t('collections.reports')}</h2>
      <p class="explain flush">
        {#if destinations.length}
          {t('collections.reportsText', { brand: store.brand.name, destinations: destinations.join(', ') })}
        {:else}
          {t('collections.noReports')}
        {/if}
      </p>
      <label class="interval">
        <span>{t('collections.send')}</span>
        <select
          value={store.settings.reporting}
          disabled={isLocked('reporting')}
          onchange={(e) => changeSettings({ reporting: e.currentTarget.value as 'off' | 'errors' | 'all' })}
        >
          <option value="off">{t('collections.sendNothing')}</option>
          <option value="errors">{t('collections.sendErrors')}</option>
          <option value="all">{t('collections.sendAll')}</option>
        </select>
        {#if isLocked('reporting')}<span class="org">{t('common.setByOrg')}</span>{/if}
      </label>
      <label class="check">
        <input
          type="checkbox"
          checked={store.settings.reportHostname}
          disabled={isLocked('reportHostname') || store.settings.reporting === 'off'}
          onchange={(e) => changeSettings({ reportHostname: e.currentTarget.checked })}
        />
        {t('collections.hostname')}
        {#if isLocked('reportHostname')}<span class="org">{t('common.setByOrg')}</span>{/if}
      </label>
      <p class="explain">
        {#each parts(t('collections.reportsQueue', { brand: store.brand.name })) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}
      </p>
    </section>
  {/if}

  {#if store.settings}
    <section class="auto" aria-labelledby="language-title">
      <h2 id="language-title">{t('collections.language')}</h2>
      <label class="interval">
        <span>{t('collections.languageLabel', { brand: store.brand.name })}</span>
        <select
          id="language"
          value={store.settings.language}
          disabled={isLocked('language')}
          onchange={(e) => changeSettings({ language: e.currentTarget.value as 'system' | Lang })}
        >
          <option value="system">{t('collections.languageSystem')}</option>
          <!-- Each language in its own words, so people find theirs whatever is shown. -->
          {#each Object.entries(languages) as [code, name]}<option value={code} lang={code}>{name}</option>{/each}
        </select>
        {#if isLocked('language')}<span class="org">{t('common.setByOrg')}</span>{/if}
      </label>
    </section>
  {/if}

  {#if store.collections.length}
    <div class="toolbar">
      <h2>{t('collections.count', { n: store.collections.length })}</h2>
      <button class="secondary" disabled={working !== null} onclick={() => update('*')}>
        {working === '*' ? t('collections.updating') : t('collections.updateAll')}
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
          <span class="mono rev" title={t('collections.revision')}>{c.revision}</span>
          <span class="count">{t('common.apps', { n: c.apps })}</span>
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
          <details class="copts">
            <summary>{t('collections.downloads')}</summary>
            <CollectionSettings collection={c} />
          </details>
        </li>
      {/each}
    </ul>
  {/if}

  <p class="about">
    furnish {version}
    · <button class="link" onclick={() => api.openLicenses().catch((e) => toast('error', t('collections.licensesFailed'), String(e)))}>{t('collections.licenses')}</button>
  </p>
</section>

<style>
  section {
    max-width: 56rem;
  }
  .about {
    margin-top: 2.5rem;
    color: var(--muted);
    font-size: 0.85rem;
  }
  .about .link {
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
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

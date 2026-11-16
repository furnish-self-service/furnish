<script lang="ts">
  // Everything furnish does on its own and how it looks: startup, updates, language, reports,
  // and what it is. One section per topic, one idea per row: what it does on the left, the
  // control on the right. Settings the admin policy locks stay visible, not editable.
  import * as api from './api';
  import { store, changeSettings, checkForUpdates, checkSelfUpdate, restartToUpdate, toast, isLocked } from './state.svelte';
  import { t, parts, formatTime } from './i18n.svelte';
  import { languages, type Lang } from './i18n.ts';

  const intervals = $derived([
    [15, t('settings.every15')],
    [30, t('settings.every30')],
    [60, t('settings.everyHour')],
    [240, t('settings.every4Hours')],
    [1440, t('settings.everyDay')],
    [0, t('settings.never')],
  ] as [number, string][]);
  // A value set outside the app (settings.json) still shows up as a choice.
  const options = $derived(
    store.settings && !intervals.some(([m]) => m === store.settings!.updateIntervalMinutes)
      ? [...intervals, [store.settings.updateIntervalMinutes, t('settings.everyMinutes', { n: store.settings.updateIntervalMinutes })]]
      : intervals,
  );

  let version = $state('');
  api.version().then((v) => (version = v));
  let dataDir = $state('');
  api.dataDir().then((d) => (dataDir = d)).catch(() => {});
  // The organization's furnish addresses that take reports; the section shows only with one.
  let destinations = $state<string[]>([]);
  api.reportDestinations().then((d) => (destinations = d));
  // On NixOS: where its configuration lives, for the NixOS section.
  let nixos = $state<api.NixosSetup | null>(null);
  // What the user types: the configuration's folder and the flake attribute. Empty: as found.
  let nixosPath = $state('');
  let nixosAttribute = $state('');
  const loadNixos = () =>
    api
      .nixosSetup()
      .then((s) => {
        nixos = s;
        nixosAttribute = s?.chosen.flakeAttribute?.setBy === 'user' ? s.chosen.flakeAttribute.value : '';
      })
      .catch(() => {});
  loadNixos();
  // Set (or with null, forget) a NixOS setting, then show what furnish finds with it.
  async function saveNixos(name: 'configuration' | 'flakeAttribute', value: string | null) {
    try {
      await api.setNixosSetting(name, value);
      if (name === 'configuration') nixosPath = '';
      await loadNixos();
    } catch (e) {
      toast('error', t('nixos.saveFailed'), String(e));
    }
  }

  const brand = $derived(store.brand.name);

  // furnish's own updates: it installs them itself only when installed from the .msi or .dmg.
  const self = $derived(store.selfUpdate);
  const updatesItself = $derived(self?.method === 'msi' || self?.method === 'app');
  const methods: Record<api.InstallMethod, string> = $derived({
    msi: t('settings.methodMsi'),
    app: t('settings.methodApp'),
    homebrew: t('settings.methodHomebrew'),
    deb: t('settings.methodDeb'),
    rpm: t('settings.methodRpm'),
    nix: t('settings.methodNix'),
    other: '',
  });
  const how = $derived.by(() => {
    switch (self?.method) {
      case 'homebrew':
        return t('settings.selfUpdateHowHomebrew');
      case 'deb':
      case 'rpm':
        return t('settings.selfUpdateHowSystem');
      case 'nix':
        return t('settings.selfUpdateHowNix');
      case 'other':
        return t('settings.selfUpdateHowOther', { brand });
      default:
        return self?.locked && self.mode === 'notify' ? t('settings.selfUpdateByOrg') : '';
    }
  });
  const busy = $derived(Object.keys(store.busy).length > 0 || store.queue.length > 0);
  function updateNow() {
    store.restartWanted = true;
    checkSelfUpdate(true);
  }
</script>

{#snippet locked(key: keyof api.Settings)}
  {#if isLocked(key)}<span class="org">{t('common.setByOrg')}</span>{/if}
{/snippet}

<section class="settings">
  <h1>{t('settings.title')}</h1>

  {#if store.warnings.length}
    <div class="notice" role="status">
      <strong>{t('settings.warnings')}</strong>
      <p>{t('settings.warningsText', { brand })}</p>
      <ul>
        {#each store.warnings as warning}<li>{warning}</li>{/each}
      </ul>
    </div>
  {/if}

  {#if store.settings}
    <section aria-labelledby="startup-title">
      <h2 id="startup-title">{t('settings.startup')}</h2>
      <ul class="rows">
        <li>
          <label class="row">
            <span class="text">
              <span class="label">{t('settings.startAtSignIn', { brand })}</span>
              <span class="desc">{t('settings.startAtSignInText', { brand })}</span>
            </span>
            <span class="control">
              {@render locked('startAtSignIn')}
              <input
                type="checkbox"
                checked={store.settings.startAtSignIn}
                disabled={isLocked('startAtSignIn')}
                onchange={(e) => changeSettings({ startAtSignIn: e.currentTarget.checked })}
              />
            </span>
          </label>
        </li>
      </ul>
    </section>

    <section aria-labelledby="updates-title">
      <h2 id="updates-title">{t('settings.updates')}</h2>
      <p class="intro">{t('settings.updatesText')}</p>
      <ul class="rows">
        <li class="status">
          <span class="text" aria-live="polite">
            {#if store.autoUpdate.running}
              <span class="label">{t('settings.checking')}</span>
            {:else if store.autoUpdate.error}
              <span class="label failed">{t('settings.lastFailed')}</span>
              <span class="desc error">{store.autoUpdate.error}</span>
            {:else if store.autoUpdate.lastAt}
              <span class="label">{t('settings.lastChecked', { time: formatTime(store.autoUpdate.lastAt) })}</span>
            {:else}
              <span class="label">{t('settings.notChecked', { brand })}</span>
            {/if}
            {#if store.autoUpdate.postponed}
              <span class="desc postponed">{t('catalog.postponed', {
                // The backend says "measuring …" until it has a first sample (src/load.rs).
                reason: store.autoUpdate.postponed.startsWith('measuring')
                  ? t('catalog.measuring')
                  : t('catalog.busy', { load: store.autoUpdate.postponed }),
              })}</span>
            {/if}
          </span>
          <span class="control">
            <button class="secondary" disabled={store.autoUpdate.running} onclick={checkForUpdates}>{t('settings.checkNow')}</button>
          </span>
        </li>
        <li class="row">
          <span class="text">
            <span class="label" id="look-label">{t('settings.lookForUpdates')}</span>
          </span>
          <span class="control stack" role="group" aria-labelledby="look-label">
            <label class="inline">
              <input
                type="checkbox"
                checked={store.settings.updateOnLaunch}
                disabled={isLocked('updateOnLaunch')}
                onchange={(e) => changeSettings({ updateOnLaunch: e.currentTarget.checked })}
              />
              {t('settings.onStart', { brand })}
              {@render locked('updateOnLaunch')}
            </label>
            <label class="inline interval">
              {t('settings.whileRunning', { brand })}
              <select
                id="interval"
                value={store.settings.updateIntervalMinutes}
                disabled={isLocked('updateIntervalMinutes')}
                onchange={(e) => changeSettings({ updateIntervalMinutes: Number(e.currentTarget.value) })}
              >
                {#each options as [minutes, text]}
                  <option value={minutes}>{text}</option>
                {/each}
              </select>
              {@render locked('updateIntervalMinutes')}
            </label>
          </span>
        </li>
        <li>
          <label class="row">
            <span class="text">
              <span class="label">{t('settings.loadAware')}</span>
              <span class="desc">{t('settings.loadAwareText')}</span>
            </span>
            <span class="control">
              {@render locked('loadAwareUpdates')}
              <input
                type="checkbox"
                checked={store.settings.loadAwareUpdates}
                disabled={isLocked('loadAwareUpdates')}
                onchange={(e) => changeSettings({ loadAwareUpdates: e.currentTarget.checked })}
              />
            </span>
          </label>
          <details class="more">
            <summary>{t('settings.busyWhat')}</summary>
            <p>
              {t('settings.busyMeans', {
                cpu: store.settings.loadLimits.cpuPercent,
                memory: store.settings.loadLimits.memoryPercent,
                network: store.settings.loadLimits.networkKbps,
                brand,
              })}
            </p>
          </details>
        </li>
      </ul>
    </section>

    <section aria-labelledby="language-title">
      <h2 id="language-title">{t('settings.language')}</h2>
      <ul class="rows">
        <li class="row">
          <label class="text" for="language">
            <span class="label">{t('settings.languageLabel', { brand })}</span>
          </label>
          <span class="control">
            {@render locked('language')}
            <select
              id="language"
              value={store.settings.language}
              disabled={isLocked('language')}
              onchange={(e) => changeSettings({ language: e.currentTarget.value as 'system' | Lang })}
            >
              <option value="system">{t('settings.languageSystem')}</option>
              <!-- Each language in its own words, so people find theirs whatever is shown. -->
              {#each Object.entries(languages) as [code, name]}<option value={code} lang={code}>{name}</option>{/each}
            </select>
          </span>
        </li>
        <li class="row">
          <label class="text" for="fallback-language">
            <span class="label">{t('settings.fallbackLanguage')}</span>
            <span class="desc">{t('settings.fallbackLanguageText')}</span>
          </label>
          <span class="control">
            {@render locked('fallbackLanguage')}
            <select
              id="fallback-language"
              value={store.settings.fallbackLanguage}
              disabled={isLocked('fallbackLanguage')}
              onchange={(e) => changeSettings({ fallbackLanguage: e.currentTarget.value })}
            >
              {#each Object.entries(languages) as [code, name]}<option value={code} lang={code}>{name}</option>{/each}
              <!-- The organization may choose a language furnish itself doesn't come in. -->
              {#if !(store.settings.fallbackLanguage in languages)}
                <option value={store.settings.fallbackLanguage}>{store.settings.fallbackLanguage}</option>
              {/if}
            </select>
          </span>
        </li>
      </ul>
    </section>

    {#if destinations.length || isLocked('reporting')}
      <section aria-labelledby="reports-title">
        <h2 id="reports-title">{t('settings.reports')}</h2>
        <p class="intro">
          {#if destinations.length}
            {t('settings.reportsText', { brand, destinations: destinations.join(', ') })}
          {:else}
            {t('settings.noReports')}
          {/if}
        </p>
        <ul class="rows">
          <li class="row">
            <label class="text" for="reporting">
              <span class="label">{t('settings.send')}</span>
            </label>
            <span class="control">
              {@render locked('reporting')}
              <select
                id="reporting"
                value={store.settings.reporting}
                disabled={isLocked('reporting')}
                onchange={(e) => changeSettings({ reporting: e.currentTarget.value as 'off' | 'errors' | 'all' })}
              >
                <option value="off">{t('settings.sendNothing')}</option>
                <option value="errors">{t('settings.sendErrors')}</option>
                <option value="all">{t('settings.sendAll')}</option>
              </select>
            </span>
          </li>
          <li>
            <label class="row">
              <span class="text">
                <span class="label">{t('settings.hostname')}</span>
              </span>
              <span class="control">
                {@render locked('reportHostname')}
                <input
                  type="checkbox"
                  checked={store.settings.reportHostname}
                  disabled={isLocked('reportHostname') || store.settings.reporting === 'off'}
                  onchange={(e) => changeSettings({ reportHostname: e.currentTarget.checked })}
                />
              </span>
            </label>
          </li>
        </ul>
        <p class="note">
          {#each parts(t('settings.reportsQueue', { brand })) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}
        </p>
      </section>
    {/if}
  {/if}

  {#if nixos}
    {@const config = nixos.configuration}
    {@const chosen = nixos.chosen}
    <section aria-labelledby="nixos-title">
      <h2 id="nixos-title">NixOS</h2>
      <p class="intro">{t('nixos.declared')}</p>
      <ul class="rows">
        <li class="row">
          <span class="text">
            <span class="label">{t('nixos.config')}</span>
            {#if config}
              <span class="desc">
                {#if config.source !== 'policy'}{t(`nixos.from.${config.source}`, { variable: config.via ?? '' })}{/if}
                {#if config.revision === 'differs'}<span class="warn">{t('nixos.differs')}</span>{/if}
                {#if config.problem}<span class="warn">{t(`nixos.problem.${config.problem}`, { path: config.path, name: nixos.flakeAttribute.value })}</span>{/if}
              </span>
            {:else}
              <span class="desc">{t('nixos.noConfig')}</span>
            {/if}
            {#if nixos.rejected}
              <span class="desc warn">{t(`nixos.problem.${nixos.rejected[1]}`, { path: nixos.rejected[0], name: nixos.flakeAttribute.value })}</span>
            {/if}
          </span>
          {#if config}
            <span class="control value">
              {#if config.source === 'policy'}<span class="org">{t('common.setByOrg')}</span>{/if}
              <code class="path">{config.path}</code> · {t(config.kind === 'flake' ? 'nixos.flake' : 'nixos.channels')}
            </span>
          {/if}
        </li>
        {#if !config && nixos.suggestions.length}
          <li class="row">
            <span class="text">
              <span class="label">{t('nixos.suggestions')}</span>
              <span class="desc">{t('nixos.suggestionsText')}</span>
            </span>
            <span class="control stack">
              {#each nixos.suggestions as s}
                <span class="inline">
                  <code class="path">{s.path}</code>
                  <button class="secondary" onclick={() => saveNixos('configuration', s.path)}>{t('nixos.useThis')}</button>
                </span>
              {/each}
            </span>
          </li>
        {/if}
        {#if !chosen.configuration?.locked}
          <li class="row">
            <label class="text" for="nixos-path">
              <span class="label">{t('nixos.setPath')}</span>
              <span class="desc">{t('nixos.setPathText')}</span>
            </label>
            <span class="control">
              <input id="nixos-path" type="text" spellcheck="false" placeholder={t('nixos.pathPlaceholder')} bind:value={nixosPath} />
              <button class="secondary" disabled={!nixosPath.trim()} onclick={() => saveNixos('configuration', nixosPath.trim())}>{t('nixos.save')}</button>
              {#if chosen.configuration?.setBy === 'user'}
                <button class="link" onclick={() => saveNixos('configuration', null)}>{t('nixos.automatic')}</button>
              {/if}
            </span>
          </li>
        {/if}
        {#if !config || config.kind === 'flake'}
          <li class="row">
            <label class="text" for="nixos-attribute">
              <span class="label">{t('nixos.flakeAttribute')}</span>
              <span class="desc">{t('nixos.flakeAttributeText')}</span>
            </label>
            <span class="control">
              {#if chosen.flakeAttribute?.locked}
                <span class="org">{t('common.setByOrg')}</span>
                <code class="path">{nixos.flakeAttribute.value}</code>
              {:else}
                <input id="nixos-attribute" type="text" spellcheck="false" placeholder={nixos.flakeAttribute.value} bind:value={nixosAttribute} />
                <button class="secondary" disabled={!nixosAttribute.trim()} onclick={() => saveNixos('flakeAttribute', nixosAttribute.trim())}>{t('nixos.save')}</button>
                {#if chosen.flakeAttribute?.setBy === 'user'}
                  <button class="link" onclick={() => saveNixos('flakeAttribute', null)}>{t('nixos.automatic')}</button>
                {/if}
              {/if}
            </span>
          </li>
        {/if}
        <li class="row">
          <span class="text">
            <span class="label">{t('nixos.homeManager')}</span>
            {#if nixos.homeManagerConfiguration}<span class="desc"><code class="path">{nixos.homeManagerConfiguration.path}</code></span>{/if}
          </span>
          <span class="control value">{t(nixos.homeManager || nixos.homeManagerConfiguration ? 'nixos.homeManagerFound' : 'nixos.homeManagerMissing')}</span>
        </li>
      </ul>
      <p class="note">
        {#each parts(t('nixos.installs')) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}
      </p>
    </section>
  {/if}

  <section aria-labelledby="about-title">
    <h2 id="about-title">{t('settings.about')}</h2>
    <ul class="rows">
      <li class="row">
        <span class="text"><span class="label">{t('settings.version')}</span></span>
        <span class="control value" id="version">furnish {version}</span>
      </li>
      {#if self && methods[self.method]}
        <li class="row">
          <span class="text"><span class="label">{t('settings.installedWith')}</span></span>
          <span class="control value" id="install-method">{methods[self.method]}</span>
        </li>
      {/if}
      {#if self && self.mode !== 'off' && (self.available || self.error)}
        <li class="status" id="self-update">
          <span class="text" aria-live="polite">
            {#if self.state === 'downloading'}
              <span class="label">{t('settings.selfUpdateDownloading', { version: self.available ?? '' })}</span>
            {:else if self.state === 'ready'}
              <span class="label new">{t('settings.selfUpdateReady', { version: self.available ?? '' })}</span>
              {#if store.restartWanted && busy}
                <span class="desc">{t('settings.selfUpdateWaiting', { brand })}</span>
              {:else if self.mode === 'auto'}
                <span class="desc">{t('settings.selfUpdateReadyText', { brand })}</span>
              {/if}
            {:else if self.available}
              <span class="label new">{t('settings.selfUpdateAvailable', { version: self.available })}</span>
              {#if how}
                <span class="desc">{#each parts(how) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}</span>
              {/if}
            {/if}
            {#if self.error}
              <span class="desc error">{t('settings.selfUpdateFailed', { brand })}: {self.error}</span>
            {/if}
          </span>
          {#if updatesItself && self.available}
            <span class="control">
              {#if self.state === 'ready'}
                <button class="secondary" disabled={store.restartWanted} onclick={restartToUpdate}>{t('settings.restartToUpdate')}</button>
              {:else if self.state === 'idle' && !(self.locked && self.mode === 'notify')}
                <button class="secondary" onclick={updateNow}>{t('settings.selfUpdateNow')}</button>
              {/if}
            </span>
          {/if}
        </li>
      {/if}
      {#if store.settings}
        <li class="row">
          <label class="text" for="self-update-mode">
            <span class="label">{t('settings.selfUpdate', { brand })}</span>
          </label>
          <span class="control">
            {@render locked('selfUpdate')}
            <select
              id="self-update-mode"
              value={updatesItself || store.settings.selfUpdate === 'off' ? store.settings.selfUpdate : 'notify'}
              disabled={isLocked('selfUpdate')}
              onchange={(e) => changeSettings({ selfUpdate: e.currentTarget.value as 'auto' | 'notify' | 'off' })}
            >
              {#if updatesItself}<option value="auto">{t('settings.selfUpdateAuto')}</option>{/if}
              <option value="notify">{t('settings.selfUpdateNotify')}</option>
              <option value="off">{t('settings.selfUpdateOff')}</option>
            </select>
          </span>
        </li>
      {/if}
      {#if dataDir}
        <li class="row">
          <span class="text"><span class="label">{t('settings.dataFolder')}</span></span>
          <span class="control value"><code class="path">{dataDir}</code></span>
        </li>
      {/if}
      <li class="row">
        <span class="text">
          <button class="link" onclick={() => api.openLicenses().catch((e) => toast('error', t('settings.licensesFailed'), String(e)))}>
            {t('settings.licenses')}
          </button>
        </span>
      </li>
    </ul>
  </section>
</section>

<style>
  .settings {
    max-width: 44rem;
  }
  h1 {
    margin: 0;
    font: 700 2rem/1.1 var(--font-display);
    letter-spacing: -0.02em;
  }
  section section {
    margin-top: 2.25rem;
  }
  h2 {
    margin: 0 0 0.5rem;
    font: 650 1rem var(--font-display);
  }
  .intro,
  .note {
    margin: 0 0 0.6rem;
    font-size: 0.85rem;
    line-height: 1.45;
    color: var(--muted);
  }
  .note {
    margin: 0.6rem 0 0;
  }
  .notice {
    margin-top: 1.25rem;
    padding: 0.8rem 1rem;
    border-radius: 12px;
    background: var(--highlight-soft);
    color: var(--ink);
    font-size: 0.85rem;
  }
  .notice p {
    margin: 0.25rem 0 0.4rem;
  }
  .notice ul {
    margin: 0;
    padding-left: 1.2rem;
    font: 0.8rem var(--font-mono);
    overflow-wrap: anywhere;
  }
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  .rows > li {
    border-bottom: 1px solid var(--line);
  }
  .row,
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem 1.5rem;
    padding: 0.75rem 0;
  }
  label.row {
    cursor: pointer;
  }
  .text {
    display: grid;
    gap: 0.15rem;
    flex: 1 1 14rem;
    min-width: 0;
  }
  .label {
    font-weight: 600;
  }
  .status .label {
    font-weight: 400;
    color: var(--ink-soft);
  }
  .status .label.new {
    font-weight: 600;
    color: var(--ink);
  }
  .status .label.failed {
    font-weight: 600;
    color: var(--danger);
  }
  .desc {
    font-size: 0.85rem;
    color: var(--muted);
  }
  .control {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    max-width: 100%;
  }
  .control.stack {
    flex-direction: column;
    align-items: flex-end;
    gap: 0.5rem;
  }
  .inline {
    display: flex;
    flex-wrap: wrap;
    max-width: 100%;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.92rem;
  }
  input[type='checkbox'] {
    width: 1.05rem;
    height: 1.05rem;
    margin: 0;
    accent-color: var(--accent);
  }
  select {
    padding: 0.45rem 0.7rem;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: 0.92rem var(--font-body);
    max-width: 100%;
    min-width: 0;
    text-overflow: ellipsis;
  }
  .value {
    color: var(--ink-soft);
    font-size: 0.92rem;
  }
  .path {
    font: 0.8rem var(--font-mono);
    overflow-wrap: anywhere;
    user-select: all;
  }
  .error {
    white-space: pre-line;
  }
  input[type='text'] {
    padding: 0.45rem 0.7rem;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: 0.85rem var(--font-mono);
    width: 16rem;
    max-width: 100%;
    min-width: 0;
  }
  .warn {
    display: block;
    color: var(--highlight-ink);
  }
  .postponed {
    font-weight: 650;
    color: var(--highlight-ink);
  }
  .more {
    margin: -0.35rem 0 0.75rem;
    font-size: 0.85rem;
    color: var(--muted);
  }
  .more summary {
    width: fit-content;
    cursor: pointer;
    font-weight: 600;
    color: var(--ink-soft);
  }
  .more p {
    margin: 0.35rem 0 0;
    line-height: 1.45;
  }
  .org {
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 650;
    color: var(--ink-soft);
    background: var(--hover);
    white-space: nowrap;
  }
  .link {
    width: fit-content;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-ink);
    font: inherit;
    font-size: 0.92rem;
    text-decoration: underline;
    cursor: pointer;
  }
  @media (max-width: 760px) {
    .control.stack {
      align-items: flex-start;
    }
  }
</style>

<script lang="ts">
  // Everything furnish does on its own and how it looks: startup, updates, language, reports,
  // and what it is. One section per topic, one idea per row: what it does on the left, the
  // control on the right. Settings the admin policy locks stay visible, not editable.
  import * as api from './api';
  import { store, changeSettings, checkForUpdates, checkSelfUpdate, restartToUpdate, toast, isLocked } from './state.svelte';
  import { t, parts, tparts, formatTime, lang } from './i18n.svelte';
  import { languages, warningText, type Lang } from './i18n.ts';

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

  let build = $state<api.BuildInfo>();
  api.buildInfo().then(info => (build = info));
  const sourceDate = $derived(build?.sourceDateEpoch != null ? new Date(build.sourceDateEpoch * 1000) : null);
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
  // The lines to add to the NixOS configuration: which of them was just copied.
  let copied = $state<string | null>(null);
  async function copySnippet(text: string) {
    await navigator.clipboard.writeText(text);
    copied = text;
    setTimeout(() => (copied = null), 1500);
  }
  // furnish installs apps into the configuration once its flake.nix has these (src/nixos_apps.rs).
  const appsLines = 'inputs.furnish.inputs.nixpkgs.follows = "nixpkgs";\ninputs.furnish.inputs.apps.url = "path:/var/lib/furnish/apps";';
  // Set (or with null, forget) a NixOS setting, then show what furnish finds with it.
  async function saveNixos(name: 'configuration' | 'flakeAttribute' | 'installInto', value: string | null) {
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
  const macDownload = $derived(self?.method === 'app');
  let cliLink = $state<'idle' | 'working' | 'installed'>('working');
  $effect(() => {
    if (macDownload) api.cliLinkInstalled().then(linked => cliLink = linked ? 'installed' : 'idle').catch(() => cliLink = 'idle');
  });
  async function installCliLink() {
    cliLink = 'working';
    try {
      await api.installCliLink();
      cliLink = 'installed';
    } catch (e) {
      cliLink = 'idle';
      toast('error', t('settings.cliLinkFailed'), String(e));
    }
  }
  function updateNow() {
    store.restartWanted = true;
    checkSelfUpdate(true);
  }
</script>

{#snippet openFolder(path: string, exists = true)}
  <button class="secondary" disabled={!exists} aria-label={t('settings.openFolderAt', { path })} onclick={() =>
    api.openDirectory(path).catch(e => toast('error', t('settings.openFolderFailed'), String(e)))}>
    {t('settings.openFolder')}
  </button>
{/snippet}

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
        {#each store.warnings as warning}
          <li>{#each tparts(...warningText(warning)) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}</li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if store.settings}
    <section aria-labelledby="startup-title">
      <h2 id="startup-title">{t('settings.startup')}</h2>
      <ul class="rows preference-list">
        <li>
          <label class="row">
            <span class="text">
              <span class="label">{t('settings.startAtSignIn', { brand })}</span>
              <span class="desc">{t('settings.startAtSignInText', { brand })}</span>
              {@render locked('startAtSignIn')}
            </span>
            <span class="control">
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
      <ul class="rows preference-list">
        <li class="status">
          <span class="text" aria-live="polite">
            {#if store.scan.running}
              <span class="label">{t('catalog.scanProgress', { done: store.scan.checked, total: store.scan.total })}</span>
            {:else if store.autoUpdate.running}
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
            <button class="secondary" disabled={store.autoUpdate.running || store.scan.running || store.loading} onclick={checkForUpdates}>{t('settings.checkNow')}</button>
          </span>
        </li>
        <li>
          <fieldset class="schedule">
            <legend>{t('settings.lookForUpdates')}</legend>
            <label class="row">
              <span class="text">{t('settings.onStart', { brand })}{@render locked('updateOnLaunch')}</span>
              <input
                type="checkbox"
                checked={store.settings.updateOnLaunch}
                disabled={isLocked('updateOnLaunch')}
                onchange={(e) => changeSettings({ updateOnLaunch: e.currentTarget.checked })}
              />
            </label>
            <label class="row interval">
              <span class="text">{t('settings.whileRunning', { brand })}{@render locked('updateIntervalMinutes')}</span>
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
            </label>
          </fieldset>
        </li>
        <li>
          <label class="row">
            <span class="text">
              <span class="label">{t('settings.loadAware')}</span>
              <span class="desc">{t('settings.loadAwareText')}</span>
              {@render locked('loadAwareUpdates')}
            </span>
            <span class="control">
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
      <ul class="rows preference-list">
        <li class="row">
          <label class="text" for="language">
            <span class="label">{t('settings.languageLabel', { brand })}</span>
            {@render locked('language')}
          </label>
          <span class="control">
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
            {@render locked('fallbackLanguage')}
          </label>
          <span class="control">
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
        <ul class="rows preference-list">
          <li class="row">
            <label class="text" for="reporting">
              <span class="label">{t('settings.send')}</span>
              {@render locked('reporting')}
            </label>
            <span class="control">
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
                {@render locked('reportHostname')}
              </span>
              <span class="control">
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
      <h2 id="nixos-title">{nixos.nixos ? 'NixOS' : 'Home Manager'}</h2>
      <p class="intro">{t(nixos.nixos ? 'nixos.declared' : 'nixos.homeOnly')}</p>
      <ul class="rows preference-list">
        {#if nixos.nixos}
        <li class="row">
          <span class="text">
            <span class="label">{t('nixos.config')}</span>
            {#if config}
              <span class="desc">
                {#if config.source !== 'policy' && !(config.source === 'module' && chosen.configuration?.locked)}{t(`nixos.from.${config.source}`, { variable: config.via ?? '' })}{/if}
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
              {#if config.source === 'policy' || (config.source === 'module' && chosen.configuration?.locked)}<span class="org">{t('common.setByOrg')}</span>{/if}
              <code class="path">{config.path}</code> · {t(config.kind === 'flake' ? 'nixos.flake' : 'nixos.channels')}
              {@render openFolder(config.path)}
            </span>
          {/if}
        </li>
        {#if config && !nixos.module?.nix.configuration && !['module', 'policy'].includes(config.source)}
          {@const snippet = `services.furnish.nix.configuration = "${config.path}";`}
          <li class="row">
            <span class="text">
              <span class="label">{t('nixos.keep')}</span>
              <span class="desc">{t('nixos.keepText')}</span>
            </span>
            <span class="control">
              <code class="path">{snippet}</code>
              <button class="secondary" onclick={() => copySnippet(snippet)}>{copied === snippet ? t('common.copied') : t('common.copy')}</button>
              {#if config.source !== 'setting'}
                <button class="secondary" onclick={() => saveNixos('configuration', config.path)}>{t('nixos.useThis')}</button>
              {/if}
            </span>
          </li>
        {/if}
        {#if config?.kind === 'flake' && !nixos.appsInput}
          <li class="row">
            <span class="text">
              <span class="label">{t('nixos.apps')}</span>
              <span class="desc">{t('nixos.appsText')}</span>
            </span>
            <span class="control">
              <code class="path snippet">{appsLines}</code>
              <button class="secondary" onclick={() => copySnippet(appsLines)}>{copied === appsLines ? t('common.copied') : t('common.copy')}</button>
            </span>
          </li>
        {/if}
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
                  {@render openFolder(s.path)}
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
        {/if}
        <li class="row">
          <span class="text">
            <span class="label">{t(nixos.nixos ? 'nixos.homeManager' : 'nixos.config')}</span>
            {#if nixos.homeManagerConfiguration}<span class="desc"><code class="path">{nixos.homeManagerConfiguration.path}</code></span>{/if}
          </span>
          <span class="control value">
            {t(nixos.homeManager || nixos.homeManagerConfiguration ? 'nixos.homeManagerFound' : 'nixos.homeManagerMissing')}
            {#if nixos.homeManagerConfiguration}{@render openFolder(nixos.homeManagerConfiguration.path)}{/if}
          </span>
        </li>
        {#each [{ label: t('nixos.systemFiles'), folder: nixos.systemAppsFolder }, { label: t('nixos.homeFiles'), folder: nixos.homeAppsFolder }] as { label, folder } (label)}
          {#if folder}
            <li class="row">
              <span class="text"><span class="label">{label}</span>
                <span class="desc">{folder.exists ? 'flake.nix · apps.json' : t('nixos.folderPending')}</span>
              </span>
              <span class="control value"><code class="path">{folder.path}</code>
                {@render openFolder(folder.path, folder.exists)}
              </span>
            </li>
          {/if}
        {/each}
        {#if nixos.homeHint}
          {@const lines = nixos.homeHint.lines.join('\n')}
          <li class="row">
            <span class="text">
              <span class="label">{t('nixos.homeApps')}</span>
              <span class="desc">{#each parts(t(`nixos.homeAppsText.${nixos.homeHint.kind}`)) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}</span>
            </span>
            <span class="control">
              <code class="path snippet">{lines}</code>
              <button class="secondary" onclick={() => copySnippet(lines)}>{copied === lines ? t('common.copied') : t('common.copy')}</button>
            </span>
          </li>
        {/if}
        {#if nixos.nixos}
        <li class="row">
          <label class="text" for="nixos-install-into">
            <span class="label">{t('nixos.installInto')}</span>
            <span class="desc">{t('nixos.installIntoText')}</span>
          </label>
          <span class="control">
            {#if chosen.installInto?.locked}<span class="org">{t('common.setByOrg')}</span>{/if}
            <select
              id="nixos-install-into"
              value={chosen.installInto?.setBy === 'user' || chosen.installInto?.locked ? chosen.installInto.value : ''}
              disabled={chosen.installInto?.locked}
              onchange={(e) => saveNixos('installInto', e.currentTarget.value || null)}
            >
              <option value="">{t('nixos.installIntoAutomatic', { place: t(`nixos.installInto.${nixos.installInto}`) })}</option>
              <option value="system">{t('nixos.installInto.system')}</option>
              <option value="home">{t('nixos.installInto.home')}</option>
            </select>
          </span>
        </li>
        {/if}
      </ul>
      <p class="note">
        {#each parts(t('nixos.installs')) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}
      </p>
    </section>
  {/if}

  <section aria-labelledby="about-title">
    <h2 id="about-title">{t('settings.about')}</h2>
    <ul class="rows preference-list">
      <li class="row">
        <span class="text"><span class="label">{t('settings.version')}</span></span>
        <span class="control value" id="version">{build ? `furnish ${build.version}` : '…'}</span>
      </li>
      {#if build}
        <li class="build-info">
          <details id="build-details">
            <summary>{t('settings.buildDetails')}</summary>
            <dl>
              {#if build.revision}
                <dt>{t('settings.buildRevision')}</dt><dd><code id="build-revision">{build.revision}</code></dd>
              {/if}
              <dt>{t('settings.buildTarget')}</dt><dd><code id="build-target">{build.target}</code></dd>
              {#if sourceDate}
                <dt>{t('settings.buildSourceDate')}</dt>
                <dd><time datetime={sourceDate.toISOString()}>{sourceDate.toLocaleString(lang(), {dateStyle: 'medium', timeStyle: 'long', timeZone: 'UTC'})}</time></dd>
              {/if}
            </dl>
          </details>
        </li>
      {/if}
      {#if self && methods[self.method]}
        <li class="row">
          <span class="text"><span class="label">{t('settings.installedWith')}</span></span>
          <span class="control value" id="install-method">{methods[self.method]}</span>
        </li>
      {/if}
      {#if macDownload}
        <li class="row">
          <span class="text">
            <span class="label">{t('settings.cliLink')}</span>
            <span class="desc">{t('settings.cliLinkText')}</span>
          </span>
          <span class="control" aria-live="polite">
            {#if cliLink === 'installed'}
              <code>/usr/local/bin/furnish</code>
            {:else}
              <button class="secondary" disabled={cliLink === 'working'} onclick={installCliLink}>{t('common.install')}</button>
            {/if}
          </span>
        </li>
      {/if}
      {#if !store.installedDesktop && self && self.mode !== 'off' && (self.available || self.error)}
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
      {#if self?.method === 'nix'}
        <li class="row">
          <span class="label">{t('settings.selfUpdate', { brand })}</span>
          <span class="control value">{#each parts(how) as p}{#if p.tag}<code>{p.text}</code>{:else}{p.text}{/if}{/each}</span>
        </li>
      {:else if store.settings}
        <li class="row">
          <label class="text" for="self-update-mode">
            <span class="label">{t('settings.selfUpdate', { brand })}</span>
            {@render locked('selfUpdate')}
          </label>
          <span class="control">
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
          <span class="control value"><code class="path">{dataDir}</code>{@render openFolder(dataDir)}</span>
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
    max-width: 56rem;
    margin-inline: auto;
  }
  h1 {
    margin: 0;
    font: var(--page-heading);
    letter-spacing: -0.035em;
  }
  section section { margin-top: 1.75rem; }
  h2 {
    margin: 0 0 .65rem;
    font: 650 1.125rem/1.3 var(--font-display);
    letter-spacing: -.015em;
  }
  .intro, .note {
    margin: -.25rem 0 .8rem;
    color: var(--ink-soft);
    font-size: var(--help-size);
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .note { margin: .55rem 0 0; }
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
    overflow-wrap: anywhere;
  }
  .notice code {
    font: 0.8rem var(--font-mono);
  }
  .schedule { margin: 0; border: 0; min-width: 0; padding: 0; }
  .schedule legend { padding: 0; font-weight: 650; }
  .schedule .row { margin-top: .5rem; min-height: 2rem; }
  label.row { cursor: pointer; }
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
  .control {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: .5rem;
    min-width: 0;
  }
  .control.stack { flex-direction: column; align-items: flex-start; }
  .inline { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem; }
  .value {
    color: var(--ink-soft);
    font-size: 0.92rem;
  }
  .build-info summary { width: fit-content; cursor: pointer; font-size: .92rem; }
  .build-info dl { display: grid; grid-template-columns: fit-content(40%) 1fr; gap: .4rem 1rem; margin: .75rem 0 .2rem; font-size: .86rem; }
  .build-info dt { color: var(--muted); }
  .build-info dd { margin: 0; overflow-wrap: anywhere; user-select: text; }
  .snippet {
    white-space: pre-wrap;
  }
  .path {
    font: 0.8rem var(--font-mono);
    overflow-wrap: anywhere;
    user-select: all;
  }
  .error {
    white-space: pre-line;
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
    margin: .4rem 0 0;
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
</style>

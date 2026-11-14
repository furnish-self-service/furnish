<script lang="ts">
  import type { AppInfo } from './api';
  import { store, uninstall, install, defaultDependencies, setAutoUpdate, isLocked } from './state.svelte';
  import DependencyList from './DependencyList.svelte';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';
  import { highlightYaml } from './highlight';
  import Plan from './Plan.svelte';
  import { describeCondition } from './conditions';

  let { app, onback }: { app: AppInfo; onback: () => void } = $props();
  let confirming = $state(false);
  let logList = $state<HTMLOListElement>();
  let planOpen = $state(false);
  // Dependencies to install along with the app; the list below lets users change optional ones.
  let chosen = $state<string[]>([]);
  $effect.pre(() => {
    chosen = defaultDependencies(app);
  });
  const autoUpdate = $derived(!store.settings?.noAutoUpdate.includes(app.key));
  const installed = $derived(status === 'installed' || status === 'needsUpdate');

  const lines = $derived(store.logs[app.key] ?? []);
  const busy = $derived(!!store.busy[app.key]);
  // Follow the log while it grows.
  $effect(() => {
    lines.length;
    if (logList) logList.scrollTop = logList.scrollHeight;
  });

  const status = $derived(store.status[app.key]);
  const statusText = $derived(
    ({
      installed: 'Installed and up to date',
      needsUpdate: 'Installed, update available',
      notInstalled: 'Not installed',
      unavailable: 'Not available on this system',
      blocked: `Needs ${store.reasons[app.key] ?? 'a missing tool'}${app.dependencies.length ? `: installing ${app.id} installs its dependencies first` : ''}`,
      error: "Couldn't check the status",
    } as Record<string, string>)[status ?? ''] ?? 'Checking status…',
  );
</script>

<section class="detail">
  <button class="back" onclick={onback}>← Back</button>

  <header>
    <AppIcon {app} size={96} />
    <div class="heading">
      <h1>{app.id}</h1>
      <p class="meta">{app.categories.join(' · ')}{app.categories.length ? ' — ' : ''}from {app.collection}</p>
      <p class="status" class:installed={status === 'installed'} class:update={status === 'needsUpdate' || status === 'blocked'}>{statusText}</p>
    </div>
    <div class="cta">
      {#if status === 'installed' && confirming}
        <p class="confirm">Uninstall {app.id} from this computer?</p>
        <div class="row">
          <button class="danger" onclick={() => { confirming = false; uninstall(app); }}>Uninstall</button>
          <button class="secondary" onclick={() => (confirming = false)}>Cancel</button>
        </div>
      {:else if status === 'installed' && !store.busy[app.key] && !store.queue.some((q) => q.key === app.key)}
        <button class="secondary large" onclick={() => (confirming = true)}>Uninstall</button>
      {:else}
        <ActionButton {app} large oninstall={() => install(app, chosen)} />
      {/if}
      {#if installed}
        <label class="auto">
          <input
            type="checkbox"
            checked={autoUpdate}
            disabled={isLocked('noAutoUpdate')}
            onchange={(e) => setAutoUpdate(app.key, e.currentTarget.checked)}
          />
          Update automatically{isLocked('noAutoUpdate') ? ' (set by your organization)' : ''}
        </label>
      {/if}
    </div>
  </header>

  <p class="description">{app.description || 'No description.'}</p>

  {#if app.dependencies.length}
    <h2>Dependencies</h2>
    <p class="lead">Installed along with {app.id}{app.dependencies.some((d) => !d.required) ? '. Deselect optional ones you don’t want.' : '.'}</p>
    <DependencyList {app} bind:chosen />
  {/if}

  <h2>What this sets up</h2>
  <ul class="resources">
    {#each app.resources as resource}
      <li>
        <span class="name">{resource.name}</span>
        <span class="mono type">{resource.type}</span>
        {#if resource.condition}
          <span class="condition">{describeCondition(resource.condition) ?? 'Only if'}</span>
          <span class="mono condition">{resource.condition}</span>
        {/if}
      </li>
    {/each}
  </ul>

  <!-- Expert details: collapsed by default, one click to reveal. -->
  <details class="expert" bind:open={planOpen}>
    <summary>Steps <span class="hint">preview and live progress</span></summary>
    <!-- Mounted only while open: the preview asks DSC, which takes a moment. -->
    {#if planOpen}<Plan {app} />{/if}
  </details>

  <details class="expert">
    <summary>
      Activity log
      {#if busy}<span class="live">live</span>{:else if lines.length}<span class="hint">{lines.length} lines</span>{/if}
    </summary>
    {#if lines.length}
      <ol class="panel log" bind:this={logList}>
        {#each lines as line}
          <li class={line.level}><span class="level">{line.level}</span><span>{line.message}</span></li>
        {/each}
      </ol>
    {:else}
      <p class="empty">Install, update or uninstall this app to see each step and the package manager's output here.</p>
    {/if}
  </details>

  <details class="expert">
    <summary>DSC configuration <span class="hint">apps/{app.id}.dsc.yaml</span></summary>
    <!-- highlightYaml escapes the source before adding markup. -->
    <pre class="panel code">{@html highlightYaml(app.source)}</pre>
  </details>
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
  .status.installed {
    color: var(--accent-ink);
  }
  .status.update {
    color: var(--highlight-ink);
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
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem 1rem;
    padding: 0.7rem 0;
    border-bottom: 1px solid var(--line);
  }
  .name {
    font-weight: 600;
  }
  .type,
  .condition {
    color: var(--muted);
    font-size: 0.8rem;
  }
  .mono {
    font-family: var(--font-mono);
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
  .live {
    margin-left: 0.5rem;
    padding: 0.05rem 0.45rem;
    border-radius: 999px;
    font: 700 0.7rem var(--font-body);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--on-accent);
    background: var(--accent);
  }
  .empty {
    color: var(--muted);
    font-size: 0.9rem;
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
  .log {
    list-style: none;
  }
  .log li {
    display: flex;
    gap: 0.9rem;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .level {
    flex: none;
    width: 3.6rem;
    text-align: right;
    opacity: 0.85;
  }
  .log .error {
    color: #ff8f86;
  }
  .log .warn {
    color: #f6c453;
  }
  .log .info {
    color: #7fdccf;
  }
  .log .output {
    color: #9db2b5;
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

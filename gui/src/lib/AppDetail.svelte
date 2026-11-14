<script lang="ts">
  import type { AppInfo } from './api';
  import { store, uninstall } from './state.svelte';
  import AppIcon from './AppIcon.svelte';
  import ActionButton from './ActionButton.svelte';

  let { app, onback }: { app: AppInfo; onback: () => void } = $props();
  let confirming = $state(false);

  const status = $derived(store.status[app.key]);
  const statusText = $derived(
    ({
      installed: 'Installed and up to date',
      needsUpdate: 'Installed, update available',
      notInstalled: 'Not installed',
      unavailable: 'Not available on this system',
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
      <p class="status" class:installed={status === 'installed'} class:update={status === 'needsUpdate'}>{statusText}</p>
    </div>
    <div class="cta">
      {#if status === 'installed' && confirming}
        <p class="confirm">Uninstall {app.id} from this computer?</p>
        <div class="row">
          <button class="danger" onclick={() => { confirming = false; uninstall(app); }}>Uninstall</button>
          <button class="secondary" onclick={() => (confirming = false)}>Cancel</button>
        </div>
      {:else if status === 'installed' && !store.busy[app.key]}
        <button class="secondary large" disabled={Object.keys(store.busy).length > 0} onclick={() => (confirming = true)}>Uninstall</button>
      {:else}
        <ActionButton {app} large />
      {/if}
    </div>
  </header>

  <p class="description">{app.description || 'No description.'}</p>

  <h2>What this sets up</h2>
  <ul class="resources">
    {#each app.resources as resource}
      <li>
        <span class="name">{resource.name}</span>
        <span class="mono type">{resource.type}</span>
        {#if resource.condition}
          <span class="mono condition">only if {resource.condition}</span>
        {/if}
      </li>
    {/each}
  </ul>
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
  .large {
    padding: 0.7rem 1.4rem;
    font-size: 1rem;
    border-radius: 999px;
  }
  .description {
    margin: 2rem 0;
    font-size: 1.05rem;
    line-height: 1.6;
    color: var(--ink-soft);
  }
  h2 {
    font: 650 1rem var(--font-display);
    margin: 0 0 0.75rem;
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
</style>

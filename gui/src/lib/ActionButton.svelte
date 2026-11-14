<script lang="ts">
  // The primary action for an app. While it runs, the button fills like a level.
  import type { AppInfo } from './api';
  import { store, install, uninstall } from './state.svelte';

  let { app, large = false }: { app: AppInfo; large?: boolean } = $props();

  const status = $derived(store.status[app.key]);
  const busy = $derived(store.busy[app.key]);
  // Only one engine call runs at a time; other buttons wait instead of queueing silently.
  const engineBusy = $derived(Object.keys(store.busy).length > 0);

  const label = $derived(
    busy === 'install'
      ? status === 'needsUpdate' ? 'Updating…' : 'Installing…'
      : busy === 'uninstall'
        ? 'Uninstalling…'
        : status === 'installed'
          ? 'Uninstall'
          : status === 'needsUpdate'
            ? 'Update'
            : 'Install',
  );
  const quiet = $derived(!busy && status === 'installed');

  function run(event: MouseEvent) {
    event.stopPropagation();
    if (status === 'installed') uninstall(app);
    else install(app);
  }
</script>

{#if status === undefined}
  <span class="checking" class:large>Checking…</span>
{:else if status === 'error'}
  <span class="checking" class:large>Status unknown</span>
{:else}
  <button
    class="action"
    class:large
    class:quiet
    class:busy={!!busy}
    disabled={engineBusy}
    title={engineBusy && !busy ? 'Waiting for the current installation to finish' : undefined}
    aria-busy={!!busy}
    onclick={run}
  >
    <span class="fill" aria-hidden="true"></span>
    <span class="label">{label}</span>
  </button>
{/if}

<style>
  .action {
    position: relative;
    overflow: hidden;
    min-width: 6.5rem;
    padding: 0.42rem 1rem;
    border: 1px solid transparent;
    border-radius: 999px;
    font: 600 0.85rem/1 var(--font-body);
    color: var(--on-accent);
    background: var(--accent);
    cursor: pointer;
    transition: background 0.15s;
  }
  .action:hover:not(:disabled) {
    background: var(--accent-strong);
  }
  .action.quiet {
    color: var(--ink);
    background: transparent;
    border-color: var(--line-strong);
  }
  .action.quiet:hover:not(:disabled) {
    background: var(--danger-soft);
    border-color: var(--danger);
    color: var(--danger);
  }
  .action:disabled:not(.busy) {
    opacity: 0.45;
    cursor: default;
  }
  .large {
    min-width: 9rem;
    padding: 0.7rem 1.4rem;
    font-size: 1rem;
  }
  .label {
    position: relative;
  }
  /* Signature: the fill rises and settles while the engine works. */
  .fill {
    position: absolute;
    inset: 0;
    transform: translateY(100%);
    background: var(--accent-strong);
  }
  .busy .fill {
    animation: level 1.6s ease-in-out infinite alternate;
  }
  .busy,
  .busy:disabled {
    opacity: 1;
    cursor: progress;
  }
  .quiet.busy {
    color: var(--on-accent);
    background: var(--accent);
  }
  @keyframes level {
    from { transform: translateY(85%); }
    to { transform: translateY(15%); }
  }
  @media (prefers-reduced-motion: reduce) {
    .busy .fill {
      animation: none;
      transform: translateY(50%);
    }
  }
  .checking {
    font-size: 0.8rem;
    color: var(--muted);
  }
</style>

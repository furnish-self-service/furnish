<script lang="ts">
  // Opens when installing an app with optional dependencies from a card: choose which to include.
  import { store, appByKey, install, defaultDependencies, appName } from './state.svelte';
  import DependencyList from './DependencyList.svelte';
  import { t } from './i18n.svelte';

  let dialog = $state<HTMLDialogElement>();
  const app = $derived(store.choosing ? appByKey(store.choosing) : undefined);
  let chosen = $state<string[]>([]);

  $effect(() => {
    if (app && dialog && !dialog.open) {
      chosen = defaultDependencies(app);
      dialog.showModal();
    }
  });

  function close() {
    store.choosing = null;
    dialog?.close();
  }
  function confirm() {
    if (app) install(app, chosen);
    close();
  }
</script>

<dialog bind:this={dialog} onclose={() => (store.choosing = null)} aria-labelledby="deps-title">
  {#if app}
    <h2 id="deps-title">{t('deps.title', { app: appName(app) })}</h2>
    <p>{t('deps.text', { app: appName(app) })}</p>
    <DependencyList {app} bind:chosen />
    <div class="actions">
      <button class="secondary" onclick={close}>{t('common.cancel')}</button>
      <button class="primary" onclick={confirm}>{t('common.install')}</button>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    width: min(32rem, calc(100vw - 2rem));
    padding: 1.5rem;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow-raised);
  }
  dialog::backdrop {
    background: rgb(10 25 28 / 0.4);
  }
  h2 {
    margin: 0;
    font: 700 1.4rem var(--font-display);
  }
  p {
    color: var(--ink-soft);
    line-height: 1.5;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 1.25rem;
  }
</style>

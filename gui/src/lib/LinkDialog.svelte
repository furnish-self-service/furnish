<script lang="ts">
  // A furnish://add?endpoint=… link: add an organization's furnish address only with the user's
  // yes, since anyone can send a link.
  import * as api from './api';
  import { store, collectionAction, refreshEndpoints, askQuestions } from './state.svelte';
  import { t, parts } from './i18n.svelte';

  let dialog = $state<HTMLDialogElement>();
  $effect(() => {
    if (store.linkOrigin && dialog && !dialog.open) dialog.showModal();
  });

  function close() {
    store.linkOrigin = null;
    dialog?.close();
  }
  async function add() {
    const origin = store.linkOrigin;
    close();
    if (!origin) return;
    const ok = await collectionAction(async () => {
      const added = await api.addCollection(origin, null);
      return added.length ? t('collections.added', { names: added.map((c) => c.name).join(', ') }) : t('collections.signedInAlready');
    }, t('link.addFailed', { origin }));
    store.signInUrl = null;
    await refreshEndpoints();
    if (ok) askQuestions(true);
  }
</script>

<dialog bind:this={dialog} onclose={() => (store.linkOrigin = null)} aria-labelledby="link-title">
  <h2 id="link-title">{t('link.title')}</h2>
  <p>
    {#each parts(t('link.text', { origin: store.linkOrigin ?? '' })) as p}{#if p.tag}<strong>{p.text}</strong>{:else}{p.text}{/if}{/each}
  </p>
  <p class="warning">{t('link.warning')}</p>
  <div class="actions">
    <button class="secondary" onclick={close}>{t('common.cancel')}</button>
    <button class="primary" onclick={add}>{t('link.add')}</button>
  </div>
</dialog>

<style>
  dialog {
    width: min(30rem, calc(100vw - 2rem));
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
    font: 700 1.35rem var(--font-display);
  }
  p {
    color: var(--ink-soft);
    line-height: 1.5;
  }
  .warning {
    font-size: 0.88rem;
    color: var(--muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 1.25rem;
  }
</style>

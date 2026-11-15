<script lang="ts">
  // A furnish://add?endpoint=… link: add an organization's furnish address only with the user's
  // yes, since anyone can send a link.
  import * as api from './api';
  import { store, collectionAction, refreshEndpoints, askQuestions } from './state.svelte';

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
      return added.length ? `Added ${added.map((c) => c.name).join(', ')}` : 'Signed in; its collections were already there';
    }, `Couldn't add ${origin}`);
    store.signInUrl = null;
    await refreshEndpoints();
    if (ok) askQuestions(true);
  }
</script>

<dialog bind:this={dialog} onclose={() => (store.linkOrigin = null)} aria-labelledby="link-title">
  <h2 id="link-title">Add your organization's apps?</h2>
  <p>
    A link wants to add <strong>{store.linkOrigin}</strong>. You'll sign in there with your organization's account, and
    its apps show up here.
  </p>
  <p class="warning">Only continue if the link comes from your organization: its apps can change this computer.</p>
  <div class="actions">
    <button class="secondary" onclick={close}>Cancel</button>
    <button class="primary" onclick={add}>Sign in and add</button>
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

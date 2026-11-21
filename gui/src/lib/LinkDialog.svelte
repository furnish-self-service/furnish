<script lang="ts">
  // A furnish://add?endpoint=… link: add an organization's furnish address only with the user's
  // yes, since anyone can send a link. The dialog shows where the user will sign in and what the
  // address does, so a lookalike address stands out.
  import * as api from './api';
  import { store, dismissLink, collectionAction, refreshEndpoints, askQuestions } from './state.svelte';
  import { t, tparts } from './i18n.svelte';

  let { url }: { url: string } = $props();
  let dialog = $state<HTMLDialogElement>();
  $effect(() => {
    if (store.link && dialog && !dialog.open) dialog.showModal();
  });
  const preview = $derived(store.link?.preview ?? null);
  const host = (origin: string) => origin.replace(/^https?:\/\//, '');

  function close() {
    dismissLink(url);
    dialog?.close();
  }
  async function add() {
    const p = preview;
    close();
    if (!p) return;
    const ok = await collectionAction(async () => {
      const added = await api.addEndpoint(p.origin);
      return added.length ? t('collections.added', { names: added.map((c) => c.name).join(', ') }) : t('collections.signedInAlready');
    }, t('link.addFailed', { origin: host(p.origin) }));
    store.signInUrl = null;
    await refreshEndpoints();
    if (ok) askQuestions(true);
  }
</script>

{#snippet marked(key: Parameters<typeof tparts>[0], params: Record<string, string>)}
  {#each tparts(key, params) as p}{#if p.tag}<strong>{p.text}</strong>{:else}{p.text}{/if}{/each}
{/snippet}

<dialog bind:this={dialog} onclose={() => dismissLink(url)} aria-labelledby="link-title">
  <h2 id="link-title">{t('link.title')}</h2>
  {#if preview}
    <p>{@render marked('link.text', { name: preview.name, origin: host(preview.origin) })}</p>
    <p class="sign-in">{@render marked('signIn.at', { host: preview.signInHost, origin: host(preview.origin) })}</p>
    <ul>
      {#if preview.collections.length}<li>{t('link.collections', { names: preview.collections.join(', ') })}</li>{/if}
      {#if preview.autoInstall.length}<li>{t('link.autoInstall', { apps: preview.autoInstall.join(', ') })}</li>{/if}
      {#if preview.reporting === 'all'}<li>{t('link.reportsAll')}</li>{:else if preview.reporting}<li>{t('link.reportsErrors')}</li>{/if}
      {#if preview.branding}<li>{t('link.branding')}</li>{/if}
    </ul>
    <p class="warning">{t('link.warning')}</p>
  {:else if store.link?.error}
    <p>{t('link.invalid')}</p>
    <p class="error">{store.link.error}</p>
  {:else}
    <p aria-busy="true">{t('link.checking')}</p>
  {/if}
  <div class="actions">
    <button class="secondary" onclick={close}>{t('common.cancel')}</button>
    {#if preview}<button class="primary" onclick={add}>{t('link.add')}</button>{/if}
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
  .sign-in {
    color: var(--ink);
  }
  ul {
    margin: 0;
    padding-left: 1.2rem;
    color: var(--ink-soft);
    line-height: 1.5;
  }
  .error {
    overflow-wrap: anywhere;
    font: 0.85rem var(--font-mono);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 1.25rem;
  }
</style>

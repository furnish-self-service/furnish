<script lang="ts">
  import { actionUrl, linkActions, type ActionLink, type LinkAction } from './action-link';
  import { t } from './i18n.svelte';

  let { kind, collection, id }: Omit<ActionLink, 'action'> = $props();
  const heading = $props.id();
  let dialog: HTMLDialogElement;
  let linkInput: HTMLInputElement;
  let action = $state<LinkAction>('install');
  let feedback = $state('');
  const url = $derived(actionUrl({ kind, collection, id, action }));

  function open() {
    action = 'install';
    feedback = '';
    dialog.showModal();
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
      feedback = t('common.copied');
    } catch {
      linkInput.focus();
      linkInput.select();
      feedback = t('share.manual');
    }
  }
</script>

<button class="secondary" aria-haspopup="dialog" onclick={open}>{t('share.open')}</button>
<dialog bind:this={dialog} aria-labelledby={heading}>
  <h2 id={heading}>{t('link.share')}</h2>
  <label>{t('share.action')}
    <select bind:value={action} onchange={() => feedback = ''}>
      {#each linkActions as verb}<option value={verb}>{t(`common.${verb}`)}</option>{/each}
    </select>
  </label>
  <label>{t('share.link')}
    <input bind:this={linkInput} readonly value={url} onfocus={(event) => event.currentTarget.select()} />
  </label>
  <p role="status">{feedback}</p>
  <div class="actions">
    <button class="secondary" onclick={() => dialog.close()}>{t('share.close')}</button>
    <button class="primary" onclick={copy}>{t('share.copyLink')}</button>
  </div>
</dialog>

<style>
  dialog {
    box-sizing: border-box;
    width: min(32rem, calc(100vw - 2rem));
    padding: 1.5rem;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow-raised);
  }
  dialog::backdrop { background: rgb(10 25 28 / 0.4); }
  h2 { margin: 0; font: 700 1.35rem var(--font-display); }
  label { display: grid; gap: .4rem; margin-top: 1rem; }
  input { min-width: 0; font-family: var(--font-mono); }
  p { min-height: 1.4em; margin: .5rem 0; color: var(--ink-soft); }
  .actions { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: .5rem; }
</style>

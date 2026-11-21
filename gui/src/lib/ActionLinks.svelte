<script lang="ts">
  import { actionUrl, linkActions, type ActionLink, type LinkAction } from './action-link';
  import { t } from './i18n.svelte';

  let { kind, collection, id, name }: Omit<ActionLink, 'action'> & { name: string } = $props();
  const heading = $props.id();
  let dialog: HTMLDialogElement;
  let preview: HTMLTextAreaElement;
  let action = $state<LinkAction>('install');
  let feedback = $state('');
  const url = $derived(actionUrl({ kind, collection, id, action }));
  const sentence = $derived(t(`share.${action}`, { name }));
  const message = $derived(`${sentence}\n${url}`);

  function open() {
    action = 'install';
    feedback = '';
    dialog.showModal();
  }

  async function copy(withMessage: boolean) {
    const value = withMessage ? message : url;
    try {
      if (withMessage && typeof ClipboardItem !== 'undefined' && navigator.clipboard?.write) {
        const anchor = document.createElement('a');
        anchor.href = url;
        anchor.textContent = sentence;
        try {
          await navigator.clipboard.write([new ClipboardItem({
            'text/plain': new Blob([value], { type: 'text/plain' }),
            'text/html': new Blob([anchor.outerHTML], { type: 'text/html' }),
          })]);
          feedback = t('common.copied');
          return;
        } catch { /* Some webviews only support plain text. */ }
      }
      await navigator.clipboard.writeText(value);
      feedback = t('common.copied');
    } catch {
      preview.focus();
      preview.setSelectionRange(withMessage ? 0 : message.length - url.length, message.length);
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
  <label>{t('share.message')}
    <textarea bind:this={preview} readonly rows="4" value={message}></textarea>
  </label>
  <p role="status">{feedback}</p>
  <div class="actions">
    <button class="secondary" onclick={() => dialog.close()}>{t('share.close')}</button>
    <button class="secondary" onclick={() => copy(false)}>{t('share.copyLink')}</button>
    <button class="primary" onclick={() => copy(true)}>{t('share.copyMessage')}</button>
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
  textarea { resize: vertical; min-height: 4rem; font: inherit; }
  p { min-height: 1.4em; margin: .5rem 0; color: var(--ink-soft); }
  .actions { display: flex; justify-content: flex-end; flex-wrap: wrap; gap: .5rem; }
</style>

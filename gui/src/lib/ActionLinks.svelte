<script lang="ts">
  import { actionUrl, formatActionLink, linkActions, type ActionLink, type LinkAction } from './action-link';
  import { t } from './i18n.svelte';

  let { kind, collection, id, name }: Omit<ActionLink, 'action'> & { name: string } = $props();
  const heading = $props.id();
  let dialog: HTMLDialogElement;
  let linkInput: HTMLTextAreaElement;
  let format = $state<'url' | 'html' | 'markdown'>('url');
  let action = $state<LinkAction>('install');
  let feedback = $state('');
  const url = $derived(actionUrl({ kind, collection, id, action }));

  const value = $derived(formatActionLink(url, format, t(`share.${action}`, { name })));

  function open() {
    action = 'install';
    format = 'url';
    feedback = '';
    dialog.showModal();
  }

  async function copy() {
    try {
      if (format === 'html' && typeof ClipboardItem !== 'undefined' && navigator.clipboard?.write) {
        try {
          await navigator.clipboard.write([new ClipboardItem({
            'text/html': new Blob([value], { type: 'text/html' }),
            'text/plain': new Blob([value], { type: 'text/plain' }),
          })]);
          feedback = t('common.copied');
          return;
        } catch { /* Webviews with plain-text-only clipboard support can still copy the markup. */ }
      }
      await navigator.clipboard.writeText(value);
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
    <select aria-label={t('share.action')} bind:value={action} onchange={() => feedback = ''}>
      {#each linkActions as verb}<option value={verb}>{t(`common.${verb}`)}</option>{/each}
    </select>
  </label>
  <label>{t('share.format')}
    <select aria-label={t('share.format')} bind:value={format} onchange={() => feedback = ''}>
      <option value="url">URL</option>
      <option value="html">HTML</option>
      <option value="markdown">Markdown</option>
    </select>
  </label>
  <label>{t('share.link')}
    <textarea bind:this={linkInput} readonly rows={format === 'html' ? 5 : 2} {value} onfocus={(event) => event.currentTarget.select()}></textarea>
  </label>
  {#if feedback}<p role="status">{feedback}</p>{/if}
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
  textarea { min-width: 0; resize: vertical; font-family: var(--font-mono); }
  p { min-height: 1.4em; margin: .5rem 0; color: var(--ink-soft); }
  .actions { margin-top: 1rem; display: flex; justify-content: flex-end; flex-wrap: wrap; gap: .5rem; }
</style>

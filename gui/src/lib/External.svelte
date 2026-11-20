<script lang="ts">
  // An app installed outside furnish: furnish leaves it to what installed it, so instead of
  // Uninstall it shows the package to remove from the NixOS or Home Manager configuration, or
  // where the program is.
  import type { External } from './api';
  import { externalNote } from './state.svelte';
  import { t } from './i18n.svelte';

  let { external }: { external: External } = $props();
  const snippet = $derived(external.options?.length ? external.options.join('\n') : external.by === 'other' ? (external.path ?? external.package) : `pkgs.${external.package}`);
  let copied = $state(false);

  async function copy() {
    await navigator.clipboard.writeText(snippet);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="external">
  <p>{externalNote(external)}</p>
  <div class="snippet">
    <code>{snippet}</code>
    <button class="secondary" onclick={copy}>{copied ? t('common.copied') : t('common.copy')}</button>
  </div>
</div>

<style>
  .external {
    display: grid;
    gap: 0.4rem;
    justify-items: end;
    max-width: 18rem;
    font-size: 0.85rem;
    color: var(--ink-soft);
    text-align: right;
  }
  p {
    margin: 0;
  }
  .snippet {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  code {
    white-space: pre-wrap;
    padding: 0.2rem 0.45rem;
    border-radius: 6px;
    background: var(--hover);
    font: 0.8rem var(--font-mono);
    color: var(--ink);
  }
  button {
    padding: 0.2rem 0.6rem;
    font-size: 0.8rem;
  }
</style>

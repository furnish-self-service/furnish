<script lang="ts">
  // An app the NixOS or Home Manager configuration installs: furnish leaves it to that
  // configuration, so instead of Uninstall it shows the package to remove there.
  import type { Declared } from './api';
  import { t } from './i18n.svelte';

  let { declared }: { declared: Declared } = $props();
  const snippet = $derived(`pkgs.${declared.package}`);
  let copied = $state(false);

  async function copy() {
    await navigator.clipboard.writeText(snippet);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="declared">
  <p>{t(declared.by === 'nixos' ? 'app.declaredNixos' : 'app.declaredHomeManager')}</p>
  <div class="snippet">
    <code>{snippet}</code>
    <button class="secondary" onclick={copy}>{copied ? t('common.copied') : t('common.copy')}</button>
  </div>
</div>

<style>
  .declared {
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

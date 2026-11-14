<script lang="ts">
  import { store, dismiss } from './state.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each store.toasts as t (t.id)}
    <div class="toast {t.kind}">
      <div class="body">
        <strong>{t.title}</strong>
        {#if t.detail}<pre>{t.detail}</pre>{/if}
      </div>
      <button aria-label="Dismiss" onclick={() => dismiss(t.id)}>×</button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 1.25rem;
    bottom: 1.25rem;
    display: grid;
    gap: 0.6rem;
    width: min(26rem, calc(100vw - 2.5rem));
    z-index: 10;
  }
  .toast {
    display: flex;
    gap: 0.75rem;
    align-items: start;
    padding: 0.8rem 0.9rem;
    border-radius: 12px;
    background: var(--surface);
    border: 1px solid var(--line);
    border-left: 4px solid var(--accent);
    box-shadow: var(--shadow-raised);
    animation: rise 0.2s ease-out;
  }
  .error {
    border-left-color: var(--danger);
  }
  .body {
    flex: 1;
    min-width: 0;
  }
  pre {
    margin: 0.4rem 0 0;
    max-height: 9rem;
    overflow: auto;
    white-space: pre-wrap;
    font: 0.75rem/1.4 var(--font-mono);
    color: var(--ink-soft);
  }
  button {
    border: 0;
    background: none;
    font-size: 1.2rem;
    line-height: 1;
    color: var(--muted);
    cursor: pointer;
  }
  @keyframes rise {
    from { opacity: 0; transform: translateY(8px); }
  }
  @media (prefers-reduced-motion: reduce) {
    .toast { animation: none; }
  }
</style>

<script lang="ts">
  import type { LogLine } from './api';
  import { t } from './i18n.svelte';

  let { lines, busy = false }: { lines: LogLine[]; busy?: boolean } = $props();
  let open = $state(false);
  let logList = $state<HTMLOListElement>();
  $effect(() => {
    lines.length;
    if (logList) logList.scrollTop = logList.scrollHeight;
  });
</script>

<details bind:open>
  <summary>
    {t('app.log')}
    {#if busy}<span class="live">{t('app.live')}</span>{:else if lines.length}<span class="hint">{t('app.lines', { n: lines.length })}</span>{/if}
  </summary>
  {#if open && lines.length}
    <ol class="panel log" bind:this={logList}>
      {#each lines as line}
        <li class={line.level}><span class="level">{line.level}</span><span>{line.message}</span></li>
      {/each}
    </ol>
  {:else if open}
    <p class="empty">{t('app.logEmpty')}</p>
  {/if}
</details>

<style>
  details {
    margin-top: 1rem;
    border-top: 1px solid var(--line);
    padding-top: 0.75rem;
  }

  summary {
    cursor: pointer;
    font: 650 0.95rem var(--font-display);
    color: var(--ink-soft);
    padding: 0.25rem 0;
  }
  summary:hover {
    color: var(--ink);
  }
  .hint {
    margin-left: 0.5rem;
    font: 400 0.78rem var(--font-mono);
    color: var(--muted);
  }
  .live {
    margin-left: 0.5rem;
    padding: 0.05rem 0.45rem;
    border-radius: 999px;
    font: 700 0.7rem var(--font-body);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--on-accent);
    background: var(--accent);
  }
  .empty {
    color: var(--muted);
    font-size: 0.9rem;
  }
  /* Terminal-style panel, dark in both themes. */
  .panel {
    margin: 0.75rem 0 0;
    padding: 0.9rem 1rem;
    max-height: 22rem;
    overflow: auto;
    border-radius: 12px;
    background: #0e1d21;
    color: #d5e5e3;
    font: 0.8rem/1.55 var(--font-mono);
  }
  .log {
    list-style: none;
  }
  .log li {
    display: flex;
    gap: 0.9rem;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .level {
    flex: none;
    width: 3.6rem;
    text-align: right;
    opacity: 0.85;
  }
  .log .error {
    color: #ff8f86;
  }
  .log .warn {
    color: #f6c453;
  }
  .log .info {
    color: #7fdccf;
  }
  .log .output {
    color: #9db2b5;
  }
</style>

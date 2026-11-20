<script lang="ts">
  // The primary action for an app. One fixed-size slot: idle it is a button (Install, Update,
  // Uninstall); while running it turns, in place, into a progress pill that fills like a level,
  // with an always-visible stop control inside it (app-store pattern: no layout shift, no
  // hover-only controls). First stop click aborts after the current step, second one forces.
  import type { AppInfo } from './api';
  import { appStatus, recheckClosed, store, requestInstall, uninstall, stop, dequeue, askQuestions, appName } from './state.svelte';
  import { t } from './i18n.svelte';

  // `oninstall` replaces the default (asking about optional dependencies, then installing).
  let { app, large = false, oninstall }: { app: AppInfo; large?: boolean; oninstall?: () => void } = $props();

  const status = $derived(appStatus(app.key));
  const busy = $derived(store.busy[app.key]);
  // Only one operation runs at a time; others wait in the queue (see the Activity page).
  const engineBusy = $derived(Object.keys(store.busy).length > 0);
  const queued = $derived(store.queue.find((q) => q.key === app.key));
  const position = $derived(store.queue.findIndex((q) => q.key === app.key) + 1);
  const stopping = $derived(busy ? store.runs[app.key]?.stopping : undefined);

  const label = $derived(
    stopping
      ? t('action.stopping')
      : busy === 'install'
        ? status === 'needsUpdate' ? t('common.updating') : t('common.installing')
        : busy === 'uninstall'
          ? t('common.uninstalling')
          : status === 'installed'
            ? t('common.uninstall')
            : status === 'needsUpdate'
              ? t('common.update')
              : t('common.install'),
  );
  const quiet = $derived(!busy && status === 'installed');
  const verb = $derived(busy === 'uninstall' ? 'uninstall' : status === 'needsUpdate' ? 'update' : 'install');

  function run(event: MouseEvent) {
    event.stopPropagation();
    if (status === 'installed') return uninstall(app);
    else if (oninstall) oninstall();
    else requestInstall(app);
  }

  function halt(event: MouseEvent) {
    event.stopPropagation();
    stop(app.key, stopping ? 'force' : 'abort');
  }
</script>

{#if status === undefined}
  <span class="checking" class:large>{t('catalog.checking')}</span>
{:else if status === 'error'}
  <span class="checking" class:large>{t('action.status')}</span>
{:else if app.unanswered.length && !busy && !queued && status !== 'installed' && status !== 'needsUpdate'}
  <!-- The collection asks something first (e.g. your role); installing waits for the answer. -->
  <button class="action" class:large onclick={(e) => { e.stopPropagation(); askQuestions(true); }}>{t('action.answerFirst')}</button>
{:else if status === 'blocked' && store.checks[app.key]?.required && !busy && !queued}
  <!-- No acceptable version is offered: the card and the app page say what is needed. -->
{:else if status === 'blocked' && !app.dependencies.some((d) => d.required || d.selected) && !busy && !queued}
  <!-- A tool this app needs is missing and the collection doesn't provide it. -->
  <span class="checking needs" class:large title={t('action.needs', { what: store.checks[app.key]?.reason ?? t('common.missingTool') })}>
    {t('action.needs', { what: (store.checks[app.key]?.reason ?? t('common.missingTool')).split(' (')[0] })}
  </span>
{:else if store.queue.some(q => q.key === app.key && q.waitingForClose?.length)}
  <button class="action" class:large onclick={(event) => { event.stopPropagation(); recheckClosed(app.key); }}>{t('queue.recheckClosed')}</button>
{:else if queued}
  <!-- Waiting: same slot, removable from the queue. -->
  <div class="action queued" class:large role="group" aria-label={t('action.queuedPosition', { app: appName(app), n: position })}>
    <span class="label">{t('common.queued')}{store.queue.length > 1 ? ` · ${position}` : ''}</span>
    <button
      class="halt"
      aria-label={t('action.removeFromQueue', { app: appName(app) })}
      title={t('action.removeFromQueueTitle')}
      onclick={(e) => { e.stopPropagation(); dequeue(app.key); }}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7m0-7l-7 7" /></svg>
    </button>
  </div>
{:else if busy}
  <!-- Running: same slot, now progress + stop. -->
  <div class="action busy" class:large role="group" aria-label={t('action.busyLabel', { label, app: appName(app) })} aria-busy="true">
    <span class="fill" aria-hidden="true"></span>
    <span class="label">{label}</span>
    <button
      class="halt"
      aria-label={stopping ? t('action.forceStop') : t('action.abort')}
      title={stopping ? t('action.interrupt') : t(`action.stopAfter.${verb}`)}
      disabled={stopping === 'force'}
      onclick={halt}
    >
      {#if stopping}
        <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="4" y="4" width="8" height="8" rx="1.5" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7m0-7l-7 7" /></svg>
      {/if}
    </button>
  </div>
{:else}
  <button
    class="action"
    class:large
    class:quiet
    title={status === 'blocked'
      ? t('action.installsFirst', { deps: app.dependencies.map((d) => d.key.split('/')[1]).join(', ') })
      : engineBusy
        ? t('action.startsAfter')
        : undefined}
    onclick={run}
  >
    <span class="label">{label}</span>
  </button>
{/if}

<style>
  /* Fixed size: the slot never changes width, whatever it says or does. */
  .action {
    flex: none; /* never squeezed by a crowded row */
    position: relative;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    /* App.svelte measures the longest label of the current language, stop control included. */
    width: max(9.25rem, var(--action-width, 0px));
    height: 2.1rem;
    padding: 0 1rem;
    border: 1px solid transparent;
    border-radius: 999px;
    font: 600 0.85rem/1 var(--font-body);
    color: var(--on-accent);
    background: var(--accent);
    cursor: pointer;
    transition: background 0.15s;
  }
  .large {
    width: max(11.75rem, var(--action-large-width, 0px));
    height: 2.75rem;
    font-size: 1rem;
  }
  button.action:hover:not(:disabled) {
    background: var(--accent-strong);
  }
  .action.quiet {
    color: var(--ink);
    background: transparent;
    border-color: var(--line-strong);
  }
  button.action.quiet:hover:not(:disabled) {
    background: var(--danger-soft);
    border-color: var(--danger);
    color: var(--danger);
  }
  button.action:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .label {
    position: relative;
    white-space: nowrap;
  }
  /* Running: leave room for the stop control on the right. */
  .busy {
    justify-content: flex-start;
    padding: 0 2.3rem 0 1rem;
    cursor: progress;
  }
  .large.busy {
    padding-right: 2.9rem;
  }
  .queued {
    justify-content: flex-start;
    padding: 0 2.3rem 0 1rem;
    color: var(--ink-soft);
    background: var(--hover);
    border: 1px dashed var(--line-strong);
    cursor: default;
  }
  .large.queued {
    padding-right: 2.9rem;
  }
  .queued .halt {
    background: rgb(0 0 0 / 0.06);
  }
  .queued .halt:hover,
  .queued .halt:focus-visible {
    background: var(--danger-soft);
    color: var(--danger);
  }
  /* Signature: the fill rises and settles while the engine works. */
  .fill {
    position: absolute;
    inset: 0;
    background: var(--accent-strong);
    animation: level 1.6s ease-in-out infinite alternate;
  }
  @keyframes level {
    from { transform: translateY(85%); }
    to { transform: translateY(15%); }
  }
  @media (prefers-reduced-motion: reduce) {
    .fill {
      animation: none;
      transform: translateY(50%);
    }
  }
  .halt {
    position: absolute;
    right: 0.3rem;
    top: 50%;
    transform: translateY(-50%);
    display: grid;
    place-items: center;
    width: 1.6rem;
    height: 1.6rem;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: rgb(255 255 255 / 0.22);
    color: inherit;
    cursor: pointer;
  }
  .large .halt {
    right: 0.4rem;
    width: 2.1rem;
    height: 2.1rem;
  }
  .halt:hover:not(:disabled),
  .halt:focus-visible {
    background: rgb(255 255 255 / 0.4);
  }
  .halt:disabled {
    opacity: 0.6;
    cursor: progress;
  }
  .halt svg {
    width: 0.95rem;
    height: 0.95rem;
    fill: currentColor;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
  }
  .checking {
    font-size: 0.8rem;
    color: var(--muted);
  }
  .needs {
    flex: none;
    font-weight: 650;
    color: var(--highlight-ink);
  }
</style>

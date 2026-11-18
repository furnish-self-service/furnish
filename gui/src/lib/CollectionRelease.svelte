<script lang="ts">
  import { previewCollectionUpdate, type CollectionPreview, type ReviewedRevisions } from './api';
  import { t } from './i18n.svelte';

  let { name, busy, apply }: { name: string; busy: boolean; apply: (r: ReviewedRevisions) => Promise<void> } = $props();
  let comparison = $state<CollectionPreview>();
  let pending = $state(false);
  let error = $state('');

  async function refresh() {
    if (pending) return;
    pending = true;
    error = '';
    comparison = undefined;
    try {
      comparison = await previewCollectionUpdate(name);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      pending = false;
    }
  }
</script>

<details data-release-preview={name} ontoggle={(event) => { if (event.currentTarget.open) void refresh(); }}>
  <summary>{t('release.heading')}</summary>
  <div class="comparison" aria-busy={pending}>
    {#if pending}
      <p role="status">{t('release.loading')}</p>
    {:else if error}
      <p role="alert">{error}</p>
      <button class="secondary" onclick={refresh}>{t('release.refresh')}</button>
    {:else if comparison}
      <dl>
        {#each [
          [t('release.current'), comparison.revisions.current, comparison.currentTags.join(', ')],
          [t('release.candidate'), comparison.revisions.candidate, comparison.candidateTags.join(', ')]
        ] as [label, commit, tags]}
          <dt>{label}</dt><dd><code title={commit}>{commit.slice(0, 12)}</code> {tags}</dd>
        {/each}
      </dl>
      {#if comparison.revisions.current === comparison.revisions.candidate}
        <p>{t('collections.isUpToDate', { name })}</p>
      {:else}
        <p class="source"><code>{comparison.reference}</code></p>
        {#each comparison.apps as app}
          <div class="app">
            <strong>{app.id}</strong> · {t(`release.${app.change}`)}
            {#if app.installed}<span> · {t('release.recorded')}</span>{/if}
            {#if app.held}<span> · {t('release.held')}</span>{/if}
            {#if app.steps.length}
              <ul>
                {#each app.steps as step}
                  <li>{t(`release.${step.change}`)} · {step.name} <code>{step.kind}</code>{#if step.removal} · {t('release.uninstall')}{/if}</li>
                {/each}
              </ul>
            {/if}
          </div>
        {:else}
          <p>{t('release.noApps')}</p>
        {/each}
        {#if comparison.otherFiles.length}
          <details class="files">
            <summary>{t('release.files', { n: comparison.otherFiles.length })}</summary>
            <ul>{#each comparison.otherFiles as file}<li><code>{file}</code></li>{/each}</ul>
          </details>
        {/if}
        <p class="note">{t('release.scope')}</p>
        <button class="primary" disabled={busy} onclick={() => comparison && apply(comparison.revisions)}>{t('release.apply')}</button>
      {/if}
      <button class="secondary" disabled={busy} onclick={refresh}>{t('release.refresh')}</button>
    {/if}
  </div>
</details>

<style>
  details { flex-basis: 100%; min-width: 0; }
  summary { cursor: pointer; font-size: 0.88rem; font-weight: 600; color: var(--ink-soft); }
  .comparison { margin: .6rem 0 .4rem; border-left: 2px solid var(--line); padding-left: 1rem; overflow-wrap: anywhere; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: .25rem .9rem; font-size: .88rem; }
  dd { margin: 0; }
  .app { margin: .75rem 0; }
  ul { margin: .3rem 0; padding-left: 1.25rem; }
  code, .note, .app span { font-size: .82rem; color: var(--ink-soft); }
  .source { margin: .35rem 0 .75rem; }
  .note { max-width: 65ch; line-height: 1.5; }
</style>

<script lang="ts">
  import { t } from './i18n.svelte';

  // Newest first. Use the first released build containing each change.
  const releases = [
    { version: '0.2.0', notes: ['changelog.userInstalls', 'changelog.channels', 'changelog.collectionTools'] },
    { version: '0.1.0-alpha.399', notes: ['changelog.cache', 'changelog.kits', 'changelog.checks'] },
    { version: '0.1.0-alpha.394', notes: ['changelog.flakes'] },
    { version: '0.1.0-alpha.393', notes: ['changelog.folders', 'changelog.documents'] },
    { version: '0.1.0-alpha.390', notes: ['changelog.steps'] },
    { version: '0.1.0-alpha.380', notes: ['changelog.homebrew'] },
  ] as const;
</script>

<details id="changelog">
  <summary>{t('changelog.title')}</summary>
  <div class="entries">
    {#each releases as release (release.version)}
      <h3>{release.version}</h3>
      <ul>{#each release.notes as note}<li>{t(note)}</li>{/each}</ul>
    {/each}
  </div>
</details>

<style>
  summary { width: fit-content; cursor: pointer; font-size: .92rem; }
  .entries { max-width: 68ch; margin-block: 1rem .25rem; }
  h3 { margin: 1.15rem 0 .4rem; font: 650 .95rem/1.4 var(--font-display); color: var(--ink); }
  ul { margin: 0; padding-inline-start: 1.25rem; color: var(--ink-soft); font-size: var(--help-size); line-height: 1.5; }
  li + li { margin-top: .3rem; }
</style>

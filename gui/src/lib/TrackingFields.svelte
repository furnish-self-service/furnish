<script lang="ts">
  import type { Tracking } from './api';
  import { t } from './i18n.svelte';

  let { value = $bindable(), disabled = false }: { value: Tracking | null; disabled?: boolean } = $props();

  function choose(mode: string) {
    if (mode === 'branch') value = { mode, name: '' };
    else if (mode === 'tag') value = { mode, pattern: '' };
    else value = mode === 'default' ? { mode } : null;
  }
</script>

<div class="fields">
  <label>
    <span>{t('tracking.source')}</span>
    <select {disabled} value={value?.mode ?? 'inherit'} onchange={event => choose(event.currentTarget.value)}>
      <option value="inherit">{t('tracking.inherit')}</option>
      <option value="default">{t('tracking.default')}</option>
      <option value="branch">{t('tracking.branch')}</option>
      <option value="tag">{t('tracking.tag')}</option>
    </select>
  </label>
  {#if value?.mode === 'branch'}
    <label>
      <span>{t('tracking.branchName')}</span>
      <input {disabled} required bind:value={value.name} placeholder="release" spellcheck="false" autocomplete="off" />
    </label>
  {:else if value?.mode === 'tag'}
    <label>
      <span>{t('tracking.pattern')}</span>
      <input {disabled} required bind:value={value.pattern} placeholder="^release-[0-9]+$" spellcheck="false" autocomplete="off" />
    </label>
    <label>
      <span>{t('tracking.order')}</span>
      <select {disabled} value={value.order ?? 'version'} onchange={event => {
        if (value?.mode === 'tag') value.order = event.currentTarget.value === 'newest' ? 'newest' : 'version';
      }}>
        <option value="version">{t('tracking.order.version')}</option>
        <option value="newest">{t('tracking.order.newest')}</option>
      </select>
    </label>
    {#if value.order === 'newest'}<p class="hint">{t('tracking.dateHint')}</p>{/if}
  {/if}
</div>

<style>
  .fields { display: flex; flex-wrap: wrap; align-items: end; gap: .65rem 1rem; }
  label { display: grid; gap: .3rem; min-width: 0; font-size: .86rem; font-weight: 600; color: var(--ink-soft); }
  select, input { box-sizing: border-box; max-width: 100%; padding: .45rem .65rem; font: .92rem var(--font-body); color: var(--ink); background: var(--surface); border: 1px solid var(--line-strong); border-radius: 9px; }
  .hint { flex-basis: 100%; margin: 0; font-size: .84rem; color: var(--muted); }
  input { font-family: var(--font-mono); }
</style>

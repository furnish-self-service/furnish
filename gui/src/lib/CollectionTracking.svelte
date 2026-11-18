<script lang="ts">
  import type { CollectionInfo, Tracking } from './api';
  import { setCollectionSetting } from './api';
  import { loadAll } from './state.svelte';
  import TrackingFields from './TrackingFields.svelte';
  import { t } from './i18n.svelte';

  let { collection }: { collection: CollectionInfo } = $props();
  const effective = $derived(collection.settings.settings.tracking);
  const source = $derived(collection.settings.trackingSource);
  const locked = $derived(collection.settings.locked.includes('tracking'));
  let draft = $state<Tracking | null>(null);
  let saving = $state(false);
  let error = $state('');
  let saved = $state(false);
  function choiceKey(value: Tracking | null) {
    return JSON.stringify(value?.mode === 'tag' ? { mode: value.mode, pattern: value.pattern, order: value.order ?? 'version' } : value);
  }
  const baseline = $derived(choiceKey(source === 'user' ? effective : null));
  const dirty = $derived(Boolean(collection.settings.trackingError) || choiceKey(draft) !== baseline);

  $effect(() => {
    // A catalog reload can replace the collection object with identical settings.
    // Keep edits while its effective choice and source stay the same.
    draft = JSON.parse(baseline);
  });

  async function apply(event: SubmitEvent) {
    event.preventDefault();
    if (saving || locked || !dirty) return;
    saving = true;
    error = '';
    saved = false;
    try {
      await setCollectionSetting(collection.name, 'tracking', draft);
      await loadAll();
      saved = true;
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="tracking">
  <p class="current">
    {#if collection.settings.trackingError}{t('tracking.invalid')}
    {:else if effective.mode === 'branch'}{t('tracking.branch')}: <code>{effective.name}</code>
    {:else if effective.mode === 'tag'}{t('tracking.tag')}: <code>{effective.pattern}</code> · {t(`tracking.order.${effective.order ?? 'version'}`)}
    {:else}{t('tracking.default')}{/if}
    <span>{#if !collection.settings.trackingError || locked}{t(`tracking.origin.${source}`)}{/if}{#if locked} · {t(source === 'default' ? 'common.setByOrg' : 'tracking.locked')}{/if}</span>
  </p>
  {#if collection.settings.trackingError}<p class="error" role="alert">{collection.settings.trackingError}</p>{/if}
  {#if !locked}
    <form onsubmit={apply} oninput={() => { error = ''; saved = false; }}>
      <TrackingFields bind:value={draft} disabled={saving} />
      <button class="secondary" disabled={saving || !dirty}>{t(saving ? 'tracking.saving' : 'tracking.apply')}</button>
    </form>
  {/if}
  {#if error}<p class="error" role="alert">{t('cs.changeFailed', { name: collection.name })} · {error}</p>{/if}
  {#if saved && !dirty}<p role="status">{t('tracking.saved')}</p>{/if}
</div>

<style>
  .tracking { padding: .4rem 0 .15rem; }
  .current { margin: .2rem 0 .6rem; display: flex; flex-wrap: wrap; gap: .25rem .5rem; align-items: baseline; }
  .current span, p[role="status"] { color: var(--muted); font-size: .85rem; }
  .current code, .error { overflow-wrap: anywhere; }
  form { display: flex; flex-wrap: wrap; align-items: end; gap: .65rem 1rem; }
  .error { color: var(--danger); }
</style>

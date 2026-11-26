<script lang="ts">
  import { appName, kitName, removalImpact } from './state.svelte';
  import { plan } from './api';
  import { lang, t, terror } from './i18n.svelte';

  let { keys, removingKit, ready = $bindable(false) }: { keys: string[]; removingKit?: string; ready?: boolean } = $props();
  const impact = $derived(removalImpact(keys, removingKit));
  const names = (values: string[]) => new Intl.ListFormat(lang()).format(values);
  const selection = $derived(JSON.stringify(keys));
  type Target = { key: string; code: 'package.removal' | 'package.unrecordedRemoval'; manager: string; name: string };
  let targets = $state<Target[]>([]);
  let failures = $state<{ key: string; reason: string }[]>([]);
  let preparing = $state(false);
  let attempt = $state(0);
  $effect(() => {
    const apps: string[] = JSON.parse(selection);
    attempt;
    let current = true;
    ready = false;
    failures = [];
    targets = [];
    preparing = true;
    async function load() {
      const found: Target[] = [];
      const errors = [];
      for (const key of apps) {
        if (!current) return;
        try {
          const steps = await plan(key, true);
          for (const step of steps) {
            for (const { code, params } of step.selectionNotes ?? []) {
              if (code === 'package.removal' || code === 'package.unrecordedRemoval') {
                found.push({ key, code, manager: String(params.manager), name: String(params.name) });
              }
            }
          }
        } catch (error) {
          errors.push({ key, reason: String(error) });
        }
      }
      if (current) {
        targets = found;
        failures = errors;
        preparing = false;
        ready = !errors.length;
      }
    }
    void load();
    return () => { current = false; ready = false; };
  });
</script>

{#each failures as failure}
  <p role="alert">{failure.key} · {t('plan.previewError', { error: terror(failure.reason) })}</p>
{/each}
{#if failures.length}
  <button class="secondary" onclick={() => attempt++}>{t('status.retry')}</button>
{/if}
{#if preparing}
  <p role="status">{t('plan.preparing')}</p>
{/if}
{#if targets.length}
  <ul>
    {#each targets as target}
      <li>{#if keys.length > 1}<strong>{target.key}</strong> · {/if}{t(`plan.note.${target.code}`, { manager: target.manager, name: target.name })}</li>
    {/each}
  </ul>
{/if}

{#if impact.apps.length || impact.kits.length}
  <div class="impact" role="status">
    {#if impact.apps.length}<p>{t('uninstall.requiredBy', { apps: names(impact.apps.map(appName)) })}</p>{/if}
    {#if impact.kits.length}<p>{t('uninstall.incompleteKits', { kits: names(impact.kits.map(kitName)) })}</p>{/if}
  </div>
{/if}

<style>
  .impact {
    padding-inline-start: 0.75rem;
    border-inline-start: 0.2rem solid var(--highlight-ink);
    text-align: start;
  }
  p { margin: 0.3rem 0; }
</style>

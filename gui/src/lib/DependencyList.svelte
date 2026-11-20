<script lang="ts">
  // An app's dependencies: required ones are locked; optional ones not yet installed can be
  // deselected. `chosen` holds the keys to install along with the app.
  import type { AppInfo } from './api';
  import { appStatus, store, appByKey, isInstalled, isPending, appName, appDescription } from './state.svelte';
  import AppIcon from './AppIcon.svelte';
  import { t } from './i18n.svelte';

  let { app, chosen = $bindable() }: { app: AppInfo; chosen: string[] } = $props();

  const unavailable = (key: string) => appStatus(key) === 'unavailable';
  const locked = (dep: AppInfo['dependencies'][number]) =>
    dep.required || isInstalled(dep.key) || isPending(dep.key) || unavailable(dep.key);

  function badge(dep: AppInfo['dependencies'][number]) {
    if (unavailable(dep.key)) return t('common.notForThisSystem');
    if (isPending(dep.key)) return t('common.queued');
    if (isInstalled(dep.key)) return t('common.installed');
    return dep.required ? t('common.required') : t('common.optional');
  }

  function toggle(key: string, on: boolean) {
    chosen = on ? [...chosen, key] : chosen.filter((k) => k !== key);
  }
</script>

<ul class="deps">
  {#each app.dependencies as dep (dep.key)}
    {@const depApp = appByKey(dep.key)}
    <li class:muted={unavailable(dep.key)}>
      <label>
        <input
          type="checkbox"
          checked={!unavailable(dep.key) && (locked(dep) || chosen.includes(dep.key))}
          disabled={locked(dep)}
          onchange={(e) => toggle(dep.key, e.currentTarget.checked)}
        />
        {#if depApp}<AppIcon app={depApp} size={32} />{/if}
        <span class="text">
          <span class="name">{depApp ? appName(depApp) : dep.key.split('/')[1]}</span>
          <span class="desc">{depApp ? appDescription(depApp) : t('deps.notFound')}</span>
        </span>
      </label>
      <span class="badge" class:required={dep.required && !isInstalled(dep.key)}>{badge(dep)}</span>
    </li>
  {/each}
</ul>

<style>
  .deps {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.55rem 0;
    border-bottom: 1px solid var(--line);
  }
  li.muted {
    opacity: 0.55;
  }
  label {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.8rem;
    cursor: pointer;
  }
  input {
    flex: none;
    width: 1.1rem;
    height: 1.1rem;
    accent-color: var(--accent);
  }
  .text {
    display: grid;
    min-width: 0;
  }
  .name {
    font-weight: 650;
  }
  .desc {
    font-size: 0.83rem;
    color: var(--muted);
    overflow-wrap: anywhere;
  }
  .badge {
    flex: none;
    padding: 0.15rem 0.6rem;
    border-radius: 999px;
    font-size: 0.75rem;
    font-weight: 650;
    color: var(--ink-soft);
    background: var(--hover);
  }
  .badge.required {
    color: var(--accent-ink);
    background: var(--accent-soft);
  }
</style>

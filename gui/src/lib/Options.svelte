<script lang="ts">
  // Options (DSC parameters) of an app or a collection. Each one either follows the level above
  // (app → collection → default) or has the user's own value; the admin policy can lock it.
  import type { Option } from './api';
  import { setOption, optionLabel, optionDescription, optionValueLabel } from './state.svelte';
  import { t } from './i18n.svelte';

  let { target, options, scope }: { target: string; options: Option[]; scope: 'app' | 'collection' } = $props();

  // A dropdown: the declared values with their labels, or yes/no.
  const collection = $derived(target.split('/')[0]);
  const choices = (o: Option) =>
    o.choices.length
      ? o.choices.map((c) => ({ value: c.value, label: optionValueLabel(collection, o, c.value, c.label) }))
      : o.type === 'bool' ? [{ value: true, label: t('common.yes') }, { value: false, label: t('common.no') }] : [];

  /** What the option follows when the user has no value of their own here. */
  function inherited(o: Option) {
    const what = scope === 'app' ? t('options.sameAsCollection') : t('options.default');
    // With an own value set, the value it would fall back to isn't known here.
    if (o.missing) return t('common.choose');
    if (o.ownValue != null) return what;
    return t(o.source === 'policy' ? 'options.inheritedFromOrg' : 'options.inherited', { what, value: optionValueLabel(collection, o, o.value) });
  }

  /** `raw` is JSON from a select, or the text of an input; empty goes back to the inherited value. */
  function change(o: Option, raw: string, json: boolean) {
    const value = raw === '' ? null : json || o.type === 'int' ? JSON.parse(raw) : raw;
    setOption(target, o.name, value);
  }
</script>

<ul class="options preference-list">
  {#each options as o (o.name)}
    <li class="row">
      <label class="text" for="opt-{target}-{o.name}">
        <span class="label">{optionLabel(collection, o)}</span>
        {#if optionDescription(collection, o)}<span class="desc">{optionDescription(collection, o)}</span>{/if}
      </label>
      {#if o.locked}
        <span class="value">{optionValueLabel(collection, o, o.value)} <span class="org">{t('common.setByOrg')}</span></span>
      {:else if choices(o).length}
        <select
          id="opt-{target}-{o.name}"
          data-option={o.name}
          value={o.ownValue == null ? '' : JSON.stringify(o.ownValue)}
          onchange={(e) => change(o, e.currentTarget.value, true)}
        >
          <option value="">{inherited(o)}</option>
          {#each choices(o) as c}<option value={JSON.stringify(c.value)}>{c.label}</option>{/each}
        </select>
      {:else}
        <input
          id="opt-{target}-{o.name}"
          data-option={o.name}
          type={o.type === 'int' ? 'number' : 'text'}
          value={o.ownValue ?? ''}
          placeholder={inherited(o)}
          onchange={(e) => change(o, e.currentTarget.value, false)}
        />
      {/if}
    </li>
  {/each}
</ul>

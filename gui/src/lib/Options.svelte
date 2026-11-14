<script lang="ts">
  // Options (DSC parameters) of an app or a collection. Each one either follows the level above
  // (app → collection → default) or has the user's own value; the admin policy can lock it.
  import type { Option } from './api';
  import { setOption } from './state.svelte';

  let { target, options, scope }: { target: string; options: Option[]; scope: 'app' | 'collection' } = $props();

  const show = (v: unknown) => (v === true ? 'Yes' : v === false ? 'No' : v == null ? 'not set' : String(v));
  const choices = (o: Option): unknown[] => (o.type === 'bool' ? [true, false] : (o.allowedValues ?? []));

  /** What the option follows when the user has no value of their own here. */
  function inherited(o: Option) {
    const what = scope === 'app' ? 'Same as the collection' : 'Default';
    // With an own value set, the value it would fall back to isn't known here.
    return o.ownValue == null ? `${what} (${show(o.value)}${o.source === 'policy' ? ', from your organization' : ''})` : what;
  }

  /** `raw` is JSON from a select, or the text of an input; empty goes back to the inherited value. */
  function change(o: Option, raw: string, json: boolean) {
    const value = raw === '' ? null : json || o.type === 'int' ? JSON.parse(raw) : raw;
    setOption(target, o.name, value);
  }
</script>

<ul class="options">
  {#each options as o (o.name)}
    <li>
      <label for="opt-{target}-{o.name}">
        <span class="label">{o.label ?? o.name}</span>
        {#if o.description}<span class="desc">{o.description}</span>{/if}
      </label>
      {#if o.locked}
        <span class="value">{show(o.value)} <span class="org">Set by your organization</span></span>
      {:else if choices(o).length}
        <select
          id="opt-{target}-{o.name}"
          data-option={o.name}
          value={o.ownValue == null ? '' : JSON.stringify(o.ownValue)}
          onchange={(e) => change(o, e.currentTarget.value, true)}
        >
          <option value="">{inherited(o)}</option>
          {#each choices(o) as c}<option value={JSON.stringify(c)}>{show(c)}</option>{/each}
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

<style>
  .options {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem 1.5rem;
    padding: 0.75rem 0;
    border-bottom: 1px solid var(--line);
  }
  label {
    display: grid;
    gap: 0.15rem;
    flex: 1;
    min-width: 14rem;
  }
  .label {
    font-weight: 600;
  }
  .desc {
    font-size: 0.85rem;
    color: var(--muted);
  }
  select,
  input {
    padding: 0.45rem 0.7rem;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: 0.92rem var(--font-body);
  }
  .value {
    font-weight: 600;
  }
  .org {
    margin-left: 0.4rem;
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    font-size: 0.72rem;
    font-weight: 650;
    color: var(--ink-soft);
    background: var(--hover);
    white-space: nowrap;
  }
</style>

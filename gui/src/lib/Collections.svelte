<script lang="ts">
  import * as api from './api';
  import { store, collectionAction } from './state.svelte';

  let url = $state('');
  let name = $state('');
  let adding = $state(false);
  /** Name of the collection being updated or removed; all-update uses '*'. */
  let working = $state<string | null>(null);
  let confirmRemove = $state<string | null>(null);

  async function add(event: SubmitEvent) {
    event.preventDefault();
    adding = true;
    const ok = await collectionAction(async () => {
      const c = await api.addCollection(url, name || null);
      return `Added ${c.name} with ${c.apps} app${c.apps === 1 ? '' : 's'}`;
    }, "Couldn't add the collection");
    adding = false;
    if (ok) url = name = '';
  }

  async function update(target: string) {
    working = target;
    const names = target === '*' ? store.collections.map((c) => c.name) : [target];
    await collectionAction(async () => {
      const changed: string[] = [];
      for (const n of names) {
        const r = await api.updateCollection(n);
        if (r.old !== r.new) changed.push(n);
      }
      return changed.length ? `Updated ${changed.join(', ')}` : 'Everything is up to date';
    }, "Couldn't update");
    working = null;
  }

  async function remove(target: string) {
    working = target;
    confirmRemove = null;
    await collectionAction(async () => {
      await api.removeCollection(target);
      return `Removed ${target}`;
    }, `Couldn't remove ${target}`);
    working = null;
  }
</script>

<section>
  <h1>Collections</h1>
  <p class="lead">
    A collection is a git repository with app definitions. Apps you installed stay installed when you
    remove their collection.
  </p>

  <form onsubmit={add}>
    <label class="url">
      <span>Git URL</span>
      <input bind:value={url} required placeholder="git@github.com:your-org/apps.git" spellcheck="false" />
    </label>
    <label class="name">
      <span>Name <em>(optional)</em></span>
      <input bind:value={name} placeholder="from the URL" spellcheck="false" />
    </label>
    <button class="primary" disabled={adding || !url.trim()}>{adding ? 'Adding…' : 'Add collection'}</button>
  </form>
  <p class="warning">Apps run with administrator rights. Only add collections from people you trust.</p>

  {#if store.collections.length}
    <div class="toolbar">
      <h2>{store.collections.length} collection{store.collections.length === 1 ? '' : 's'}</h2>
      <button class="secondary" disabled={working !== null} onclick={() => update('*')}>
        {working === '*' ? 'Updating…' : 'Update all'}
      </button>
    </div>
    <ul>
      {#each store.collections as c (c.name)}
        <li>
          <div class="info">
            <span class="cname">{c.name}</span>
            <span class="mono">{c.url}</span>
          </div>
          <span class="mono rev" title="Revision">{c.revision}</span>
          <span class="count">{c.apps} app{c.apps === 1 ? '' : 's'}</span>
          {#if confirmRemove === c.name}
            <span class="confirm">
              Remove {c.name}?
              <button class="danger" onclick={() => remove(c.name)}>Remove</button>
              <button class="secondary" onclick={() => (confirmRemove = null)}>Cancel</button>
            </span>
          {:else}
            <span class="actions">
              <button class="secondary" disabled={working !== null} onclick={() => update(c.name)}>
                {working === c.name ? 'Updating…' : 'Update'}
              </button>
              <button class="secondary" disabled={working !== null} onclick={() => (confirmRemove = c.name)}>Remove</button>
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  section {
    max-width: 56rem;
  }
  h1 {
    margin: 0;
    font: 700 2rem/1.1 var(--font-display);
    letter-spacing: -0.02em;
  }
  .lead {
    color: var(--ink-soft);
    max-width: 38rem;
    line-height: 1.5;
  }
  form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    align-items: end;
    margin-top: 1.5rem;
  }
  label {
    display: grid;
    gap: 0.3rem;
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--ink-soft);
  }
  label em {
    font-weight: 400;
    color: var(--muted);
  }
  .url {
    flex: 3 1 18rem;
  }
  .name {
    flex: 1 1 9rem;
  }
  .warning {
    font-size: 0.8rem;
    color: var(--muted);
  }
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 2.5rem;
  }
  h2 {
    margin: 0;
    font: 650 1rem var(--font-display);
  }
  ul {
    margin: 0.75rem 0 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 1.25rem;
    padding: 0.85rem 0;
    border-bottom: 1px solid var(--line);
  }
  .info {
    display: grid;
    gap: 0.2rem;
    flex: 1 1 16rem;
    min-width: 0;
  }
  .cname {
    font-weight: 650;
  }
  .mono {
    font: 0.78rem var(--font-mono);
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .count {
    font-size: 0.85rem;
    color: var(--ink-soft);
  }
  .actions,
  .confirm {
    display: flex;
    gap: 0.4rem;
    align-items: center;
    font-size: 0.85rem;
  }
</style>

<script lang="ts">
  // A collection's required options ("What do you work on?"), asked when it is added and on each
  // launch until answered. "Later" postpones: that collection's apps can't be installed meanwhile.
  import type { Option } from './api';
  import { store, unansweredCollections, answerQuestions, postponeQuestions } from './state.svelte';

  let dialog = $state<HTMLDialogElement>();
  const collections = $derived(store.asking ? unansweredCollections() : []);
  /** Answers so far, as JSON (select values), by collection and option. */
  let answers = $state<Record<string, Record<string, string>>>({});

  $effect(() => {
    if (collections.length && dialog && !dialog.open) {
      answers = Object.fromEntries(collections.map((c) => [c.name, {}]));
      dialog.showModal();
    }
  });

  const choices = (o: Option) =>
    o.choices.length ? o.choices : o.type === 'bool' ? [{ value: true, label: 'Yes' }, { value: false, label: 'No' }] : [];
  const complete = $derived(collections.every((c) => c.options.filter((o) => o.missing).every((o) => answers[c.name]?.[o.name])));

  function close() {
    postponeQuestions();
    dialog?.close();
  }
  async function save() {
    const parsed = collections.flatMap((c) =>
      c.options
        .filter((o) => o.missing && answers[c.name]?.[o.name])
        .map((o) => {
          const raw = answers[c.name][o.name];
          const value = choices(o).length || o.type === 'int' ? JSON.parse(raw) : raw;
          return { collection: c.name, name: o.name, value };
        }),
    );
    store.asking = false; // answered, not postponed
    dialog?.close();
    await answerQuestions(parsed);
  }
</script>

<dialog bind:this={dialog} onclose={() => store.asking && postponeQuestions()} aria-labelledby="questions-title">
  <h2 id="questions-title">{collections.length === 1 ? `${collections[0].name} asks` : 'A few questions'}</h2>
  <p>Your answers decide what is recommended and how apps are set up. You can change them later on the Collections page.</p>
  {#each collections as c (c.name)}
    <section>
      {#if collections.length > 1}
        <h3>
          {#if c.icon}<img src={c.icon} alt="" width="22" height="22" />{/if}
          {c.name}
        </h3>
      {:else if c.icon}
        <img class="avatar" src={c.icon} alt="" width="40" height="40" />
      {/if}
      {#each c.options.filter((o) => o.missing) as o (o.name)}
        <label>
          <span class="label">{o.label ?? o.name}</span>
          {#if o.description}<span class="desc">{o.description}</span>{/if}
          {#if choices(o).length}
            <select data-question={o.name} bind:value={answers[c.name][o.name]}>
              <option value="" disabled>Choose…</option>
              {#each choices(o) as choice}
                <option value={JSON.stringify(choice.value)}>{choice.label ?? String(choice.value)}</option>
              {/each}
            </select>
          {:else}
            <input data-question={o.name} type={o.type === 'int' ? 'number' : 'text'} bind:value={answers[c.name][o.name]} />
          {/if}
        </label>
      {/each}
    </section>
  {/each}
  <div class="actions">
    <button class="secondary" onclick={close}>Later</button>
    <button class="primary" disabled={!complete} onclick={save}>Save answers</button>
  </div>
</dialog>

<style>
  dialog {
    width: min(32rem, calc(100vw - 2rem));
    padding: 1.5rem;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow-raised);
  }
  dialog::backdrop {
    background: rgb(10 25 28 / 0.4);
  }
  h2 {
    margin: 0;
    font: 700 1.4rem var(--font-display);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 1.25rem 0 0.25rem;
    font: 650 1rem var(--font-display);
  }
  h3 img,
  .avatar {
    border-radius: 8px;
  }
  .avatar {
    display: block;
    margin-top: 1rem;
  }
  p {
    color: var(--ink-soft);
    line-height: 1.5;
  }
  label {
    display: grid;
    gap: 0.3rem;
    margin-top: 1rem;
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
    padding: 0.5rem 0.7rem;
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    background: var(--surface);
    color: var(--ink);
    font: 0.95rem var(--font-body);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 1.5rem;
  }
</style>

<script lang="ts">
  import Description from './Description.svelte';
  // A collection's required options ("What do you work on?"), asked when it is added and on each
  // launch until answered. "Later" postpones: that collection's apps can't be installed meanwhile.
  import FolderInput from './FolderInput.svelte';
  import type { Option } from './api';
  import { store, unansweredCollections, answerQuestions, postponeQuestions, optionLabel, optionDescription, choiceLabel } from './state.svelte';
  import { t } from './i18n.svelte';

  let dialog = $state<HTMLDialogElement>();
  const collections = $derived(store.asking ? unansweredCollections() : []);
  /** Answers so far, as JSON (select values), by collection and option. */
  let answers = $state<Record<string, Record<string, string>>>({});

  $effect(() => {
    if (collections.length && dialog && !dialog.open) {
      // Start at "Choose…" (''): WebKit otherwise shows the first answer as if it were chosen.
      answers = Object.fromEntries(
        collections.map((c) => [c.name, Object.fromEntries(c.options.filter((o) => o.missing).map((o) => [o.name, '']))]),
      );
      dialog.showModal();
    }
  });

  const choices = (o: Option) =>
    o.choices.length ? o.choices : o.type === 'bool' ? [{ value: true, label: t('common.yes') }, { value: false, label: t('common.no') }] : [];
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
  <h2 id="questions-title">
    {#if collections.length === 1 && collections[0].icon}
      <picture>
        {#if collections[0].iconDark}<source media="(prefers-color-scheme: dark)" srcset={collections[0].iconDark} />{/if}
        <img src={collections[0].icon} alt="" width="32" height="32" />
      </picture>
    {/if}
    {collections.length === 1 ? t('questions.asks', { collection: collections[0].name }) : t('questions.title')}
  </h2>
  <p>{t('questions.text')}</p>
  {#each collections as c (c.name)}
    <section>
      {#if collections.length > 1}
        <h3>
          {#if c.icon}
            <picture>
              {#if c.iconDark}<source media="(prefers-color-scheme: dark)" srcset={c.iconDark} />{/if}
              <img src={c.icon} alt="" width="22" height="22" />
            </picture>
          {/if}
          {c.name}
        </h3>
      {/if}
      {#each c.options.filter((o) => o.missing) as o (o.name)}
        {@const field = `question-${c.name}-${o.name}`}
        <div class="question">
          <label class="label" for={field}>{optionLabel(c.name, o)}</label>
          {#if optionDescription(c.name, o)}<div class="desc"><Description text={optionDescription(c.name, o)} collection={c.name} /></div>{/if}
          {#if choices(o).length}
            <select id={field} data-question={o.name} bind:value={answers[c.name][o.name]}>
              <option value="" disabled>{t('common.choose')}</option>
              {#each choices(o) as choice}
                <option value={JSON.stringify(choice.value)}>{choiceLabel(c.name, o, choice.value, choice.label) ?? String(choice.value)}</option>
              {/each}
            </select>
          {:else if o.type === 'string' && o.folder}
            <FolderInput id={field} question={o.name} value={answers[c.name]?.[o.name] ?? ''}
              onvalue={(path) => answers[c.name][o.name] = path} />
          {:else}
            <input id={field} data-question={o.name} type={o.type === 'int' ? 'number' : 'text'} bind:value={answers[c.name][o.name]} />
          {/if}
        </div>
      {/each}
    </section>
  {/each}
  <div class="actions">
    <button class="secondary" onclick={close}>{t('questions.later')}</button>
    <button class="primary" disabled={!complete} onclick={save}>{t('questions.save')}</button>
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
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin: 0;
    font: 700 1.4rem var(--font-display);
  }
  h2 img {
    border-radius: 8px;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 1.25rem 0 0.25rem;
    font: 650 1rem var(--font-display);
  }
  h3 img {
    border-radius: 6px;
  }
  p {
    color: var(--ink-soft);
    line-height: 1.5;
  }
  .question, label {
    display: grid;
    gap: 0.3rem;
  }
  .question { margin-top: 1rem; }
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

<script lang="ts">
  import { chooseFolder } from './api';
  import { t, terror } from './i18n.svelte';
  import { toast } from './state.svelte';

  let { id, value, inherited = '', placeholder = '', option, question, onvalue }: {
    id: string; value: string; inherited?: string; placeholder?: string;
    option?: string; question?: string; onvalue: (path: string) => void;
  } = $props();
  let input: HTMLInputElement;
  let choosing = $state(false);

  async function browse() {
    choosing = true;
    const field = input;
    const originalId = id;
    try {
      const path = await chooseFolder(field.value || inherited, t('options.chooseFolder'));
      if (path !== null && field.isConnected && originalId === id) onvalue(path);
    } catch (error) {
      toast('error', t('options.chooseFolder'), terror(String(error)));
    } finally {
      choosing = false;
    }
  }
</script>

<div class="folder-field">
  <input bind:this={input} {id} type="text" {value} {placeholder} data-option={option}
    data-question={question} oninput={question ? (event) => onvalue(event.currentTarget.value) : undefined}
    onchange={question ? undefined : (event) => onvalue(event.currentTarget.value)} />
  <button type="button" class="secondary" disabled={choosing} onclick={browse}>{t('options.chooseFolder')}</button>
</div>

<style>
  .folder-field { display: flex; align-items: center; flex-wrap: wrap; gap: .4rem; }
  input { flex: 1 1 10rem; min-width: 0; }
</style>

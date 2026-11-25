<script lang="ts">
  import { tick } from 'svelte';
  import { descriptionImage, openDescriptionLink } from './api';
  import { renderDescription } from './markdown';
  import { store, toast } from './state.svelte';

  let { text, collection, compact = false }: { text: string | null | undefined; collection: string; compact?: boolean } = $props();
  let body = $state<HTMLDivElement>();
  const html = $derived(renderDescription(text ?? '', compact));

  $effect(() => {
    const name = collection;
    if (!body || compact || !html.includes('data-collection-image=')) return;
    // A pull can replace the image at the same path without changing the Markdown.
    store.collections.find((c) => c.name === name)?.revision;
    const element = body;
    let active = true;
    tick().then(() => {
      if (!active) return;
      for (const image of element.querySelectorAll<HTMLImageElement>('img[data-collection-image]')) {
        image.removeAttribute('src');
        descriptionImage(name, image.dataset.collectionImage!).then((url) => {
          if (active && url) image.src = url;
        }).catch(() => {}); // An unavailable image retains its alternative text.
      }
    });
    return () => { active = false; };
  });

  function follow(event: MouseEvent) {
    const link = event.target instanceof Element && event.target.closest('a[href]');
    if (!link) return;
    event.preventDefault();
    if (event.button <= 1) openDescriptionLink(link.getAttribute('href')!).catch((error) => toast('error', String(error)));
  }
  function links(node: HTMLElement) {
    node.addEventListener('click', follow);
    node.addEventListener('auxclick', follow);
    return { destroy() {
      node.removeEventListener('click', follow);
      node.removeEventListener('auxclick', follow);
    } };
  }
</script>

{#if compact}
  <span class="markdown">{@html html}</span>
{:else}
  <div class="markdown prose" bind:this={body} use:links>{@html html}</div>
{/if}

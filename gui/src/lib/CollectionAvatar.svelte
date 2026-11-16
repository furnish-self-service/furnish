<script lang="ts">
  // A collection's avatar (`icon` in its collection.yaml); nothing if it has none.
  import { store } from './state.svelte';

  let { name, size = 18 }: { name: string; size?: number } = $props();
  const collection = $derived(store.collections.find((c) => c.name === name));
</script>

{#if collection?.icon}
  <picture>
    {#if collection.iconDark}<source media="(prefers-color-scheme: dark)" srcset={collection.iconDark} />{/if}
    <img class="collection-avatar" src={collection.icon} alt="" width={size} height={size} />
  </picture>
{/if}

<style>
  img {
    flex: none;
    border-radius: 25%;
    vertical-align: middle;
  }
</style>

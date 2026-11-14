<script lang="ts">
  // A collection's avatar (`icon` in its collection.yaml); nothing if it has none.
  import { store } from './state.svelte';

  let { name, size = 18 }: { name: string; size?: number } = $props();
  const icon = $derived(store.collections.find((c) => c.name === name)?.icon);
</script>

{#if icon}<img class="collection-avatar" src={icon} alt="" width={size} height={size} />{/if}

<style>
  img {
    flex: none;
    border-radius: 25%;
    vertical-align: middle;
  }
</style>

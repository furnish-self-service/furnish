<script lang="ts">
  // An organization's furnish address the user isn't (or no longer) signed in to: its apps wait
  // for the sign-in. While signing in, the sign-in page's address in case the browser didn't open.
  import { store, signIn } from './state.svelte';

  const signedOut = $derived(store.endpoints.filter((e) => !e.signedIn));
  let busy = $state<string | null>(null);

  async function start(origin: string) {
    busy = origin;
    await signIn(origin);
    busy = null;
  }
</script>

{#if store.signInUrl}
  <div class="banner" role="status">
    <span>Finish signing in in your browser. If it didn't open, copy this address into it:</span>
    <code>{store.signInUrl}</code>
  </div>
{:else}
  {#each signedOut as e (e.origin)}
    <div class="banner" role="status">
      <span><strong>{e.name}</strong>: sign in to get your organization's apps and updates.</span>
      <button class="primary" disabled={busy !== null} onclick={() => start(e.origin)}>{busy === e.origin ? 'Signing in…' : 'Sign in'}</button>
    </div>
  {/each}
{/if}

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.6rem 1rem;
    margin-bottom: 1.5rem;
    padding: 0.8rem 1rem;
    border-radius: 12px;
    background: var(--highlight-soft);
    color: var(--ink);
    font-size: 0.92rem;
  }
  code {
    flex-basis: 100%;
    overflow-wrap: anywhere;
    font: 0.8rem var(--font-mono);
    color: var(--ink-soft);
    user-select: all;
  }
</style>

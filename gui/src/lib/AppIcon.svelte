<script lang="ts">
  import type { AppInfo } from './api';

  let { app, size = 56 }: { app: AppInfo; size?: number } = $props();

  // Stable hue per app, so its fallback tile is recognizable across views.
  const hue = $derived([...app.id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7));
</script>

{#if app.icon}
  <img class="icon" src={app.icon} alt="" width={size} height={size} style:--size="{size}px" />
{:else}
  <div class="icon letter" aria-hidden="true" style:--size="{size}px" style:--hue={hue}>
    {app.id.charAt(0).toUpperCase()}
  </div>
{/if}

<style>
  .icon {
    width: var(--size);
    height: var(--size);
    border-radius: calc(var(--size) * 0.24);
    flex: none;
    object-fit: contain;
  }
  .letter {
    display: grid;
    place-items: center;
    font-family: var(--font-display);
    font-weight: 700;
    font-size: calc(var(--size) * 0.46);
    color: hsl(var(--hue) 45% 28%);
    background: hsl(var(--hue) 55% 88%);
  }
  @media (prefers-color-scheme: dark) {
    .letter {
      color: hsl(var(--hue) 60% 82%);
      background: hsl(var(--hue) 30% 24%);
    }
  }
</style>

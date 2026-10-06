<script lang="ts">
  import { untrack } from "svelte";
  import { versionTitle } from "./baseText";

  let { version, updatedAt }: { version: number; updatedAt: number } = $props();

  // Highlighted for a moment when a new version settles.
  let bumped = $state(false);
  let shown = untrack(() => version);
  $effect(() => {
    if (version === shown) return;
    shown = version;
    bumped = true;
    const timer = setTimeout(() => (bumped = false), 1500);
    return () => clearTimeout(timer);
  });
</script>

<span class="version mono" class:bumped title={versionTitle(version, updatedAt)} data-tauri-drag-region>v{version}</span
>

<style>
  .version {
    padding: 2px 6px;
    border-radius: var(--radius);
    color: var(--text-2);
    font-size: var(--fs-mono);
  }
  .bumped {
    background: var(--toggle);
    color: var(--text);
  }
</style>

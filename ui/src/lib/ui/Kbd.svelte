<script lang="ts">
  // The keys that run something, set in mono beside the thing they run; `accent` marks keys that are not
  // the default ones.
  let { keys, accent = false }: { keys: string; accent?: boolean } = $props();
</script>

<kbd class:accent>{keys}</kbd>

<style>
  kbd {
    flex: none;
    color: var(--text-3);
    font: var(--fs-mono-s) var(--font-mono);
    letter-spacing: 0.02em;
  }
  .accent {
    color: var(--accent);
  }
</style>

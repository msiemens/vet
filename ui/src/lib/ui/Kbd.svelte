<script lang="ts">
  // The keys that run something, set in mono beside the thing they run.
  let { keys }: { keys: string } = $props();
</script>

<kbd>{keys}</kbd>

<style>
  kbd {
    flex: none;
    color: var(--text-3);
    font: var(--fs-mono-s) var(--font-mono);
    letter-spacing: 0.02em;
  }
</style>

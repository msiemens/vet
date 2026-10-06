<script lang="ts">
  import IconButton from "./IconButton.svelte";

  let {
    message,
    ondismiss,
  }: {
    message: string;
    ondismiss: () => void;
  } = $props();
</script>

<div class="banner" role="status">
  <span>{message}</span>
  <IconButton icon="close" label="Dismiss" size={20} iconSize={12} onclick={ondismiss} />
</div>

<style>
  .banner {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 6px 4px 12px;
    background: color-mix(in srgb, var(--old-base) 14%, var(--bg-panel));
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-ctl);
  }
  span {
    flex: 1;
  }
</style>

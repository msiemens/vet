<script lang="ts">
  import IconButton from "./IconButton.svelte";

  let {
    message,
    onclose,
    duration = 6000,
  }: {
    message: string;
    onclose: () => void;
    /** Milliseconds until it closes itself; key the toast by its message to restart this for a new one. */
    duration?: number;
  } = $props();

  $effect(() => {
    const timer = setTimeout(onclose, duration);
    return () => clearTimeout(timer);
  });
</script>

<div class="toast" role="status">
  <span>{message}</span>
  <IconButton icon="close" label="Dismiss" size={20} iconSize={12} onclick={onclose} />
</div>

<style>
  .toast {
    position: fixed;
    left: 50%;
    bottom: calc(var(--h-footer) + 12px);
    transform: translateX(-50%);
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 6px 6px 12px;
    background: var(--bg-sheet);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
    font-size: var(--fs-ctl);
  }
</style>

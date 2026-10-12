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

<!-- A popover is in the top layer, so a toast shows over a modal dialog (Settings) too. -->
<div class="surface toast" role="status" popover="manual" {@attach (toast) => void toast.showPopover()}>
  <span>{message}</span>
  <IconButton icon="close" label="Dismiss" size={20} iconSize={12} onclick={onclose} />
</div>

<style>
  .toast {
    position: fixed;
    inset: auto;
    margin: 0;
    left: 50%;
    bottom: calc(var(--h-footer) + 12px);
    transform: translateX(-50%);
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 6px 6px 12px;
    font-size: var(--fs-ctl);
  }
</style>

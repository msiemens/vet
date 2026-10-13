<script lang="ts">
  import { HoldTimer, holdWhileUsed } from "./holdTimer";
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

  // It stays while the viewer hovers or selects it.
  let timer: HoldTimer | undefined;
  $effect(() => {
    const own = new HoldTimer(onclose, duration);
    timer = own;
    return () => own.stop();
  });
</script>

<!-- A popover is in the top layer, so a toast shows over a modal dialog (Settings) too. -->
<div
  class="surface toast"
  role="status"
  popover="manual"
  {@attach (toast) => void toast.showPopover()}
  {@attach holdWhileUsed(
    () => timer?.hold(),
    () => timer?.release(),
  )}
>
  <span class="selectable">{message}</span>
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

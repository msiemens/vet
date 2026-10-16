<script lang="ts">
  import Dialog from "../../lib/ui/Dialog.svelte";
  import { DISCARD_BODY, discardTitle } from "../../state/discard";

  let {
    count,
    onkeep,
    ondiscard,
  }: {
    /** How many comment boxes hold text. */
    count: number;
    /** Escape, a press outside and the default button. */
    onkeep: () => void;
    ondiscard: () => void;
  } = $props();
</script>

<Dialog title={discardTitle(count)} onclose={onkeep} width="380px" closeButton={false}>
  <p>{DISCARD_BODY}</p>
  {#snippet actions()}
    <button
      class="ctl primary"
      type="button"
      onclick={onkeep}
      {@attach (button: HTMLElement) => {
        // The dialog takes the focus when it opens, after this button is made.
        queueMicrotask(() => button.focus());
      }}>Keep editing</button
    >
    <button class="ctl destructive" type="button" onclick={ondiscard}>Discard</button>
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
    color: var(--text-2);
  }
</style>

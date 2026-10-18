<script lang="ts">
  import Dialog from "../../lib/ui/Dialog.svelte";
  import { removeQuestion } from "../../state/removal";

  let {
    name,
    lost,
    oncancel,
    onremove,
  }: {
    name: string;
    /** What removing the project loses, one line each. */
    lost: readonly string[];
    /** Escape, a press outside and the default button. */
    oncancel: () => void;
    onremove: () => void;
  } = $props();
</script>

<Dialog title="Remove project" onclose={oncancel} width="380px" closeButton={false}>
  <p>{removeQuestion(name, lost)}</p>
  {#if lost.length}
    <ul>
      {#each lost as item (item)}<li>{item}</li>{/each}
    </ul>
  {/if}
  {#snippet actions()}
    <button
      class="ctl primary"
      type="button"
      onclick={oncancel}
      {@attach (button: HTMLElement) => {
        // The dialog takes the focus when it opens, after this button is made.
        queueMicrotask(() => button.focus());
      }}>Cancel</button
    >
    <button class="ctl destructive" type="button" onclick={onremove}>Remove</button>
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
  }
  ul {
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
    color: var(--text-2);
  }
</style>

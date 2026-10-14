<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";

  let {
    title,
    busy = false,
    onclose,
    onsubmit,
    children,
    actions,
  }: {
    title: string;
    /** While set, Escape does not close the sheet. */
    busy?: boolean;
    onclose: () => void;
    onsubmit: () => void;
    /**
     * The fields: a `label` (or `fieldset`) holds a name in a `span` (or `legend`) and its control; a
     * `.field-row` likewise holds a name and a value that is no control.
     */
    children: Snippet;
    /** The buttons, right-aligned under the fields. */
    actions: Snippet;
  } = $props();

  let sheet: HTMLDialogElement;

  onMount(() => {
    sheet.showModal();
    sheet.querySelector<HTMLElement>("input:not([type=radio]):not([disabled]), select")?.focus();
  });
</script>

<!-- A modal of its own over the settings dialog, so that Escape and Tab belong to the sheet and not to the dialog under it. -->
<dialog
  class="surface"
  aria-label={title}
  bind:this={sheet}
  oncancel={(e) => {
    e.preventDefault();
    if (!busy) onclose();
  }}
>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      onsubmit();
    }}
  >
    <h3>{title}</h3>
    {@render children()}
    <div class="actions">{@render actions()}</div>
  </form>
</dialog>

<style>
  dialog {
    width: 420px;
    max-width: calc(100vw - 32px);
    margin: 6vh auto auto;
    padding: 16px;
    color: inherit;
    font-size: var(--fs-ctl);
  }
  dialog::backdrop {
    background: transparent;
  }
  dialog:focus-visible {
    outline: none;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: var(--fs);
    font-weight: 600;
  }
  form > :global(label),
  form > :global(fieldset),
  form > :global(.field-row) {
    display: grid;
    grid-template-columns: var(--w-facts-label) 1fr;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 0;
    border: 0;
    color: var(--text-2);
  }
  form > :global(fieldset) {
    align-items: start;
  }
  form > :global(fieldset > legend) {
    float: left;
    width: var(--w-facts-label);
    padding: 0;
    padding-top: 2px;
  }
  form :global(.hint) {
    color: var(--text-3);
    font-size: var(--fs-mono);
  }
  form :global(p) {
    margin: 0;
  }
  form :global(.problem) {
    color: var(--del-text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>

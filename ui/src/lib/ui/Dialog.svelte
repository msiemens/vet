<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import IconButton from "./IconButton.svelte";

  let {
    title,
    onclose,
    width = "520px",
    padded = true,
    children,
    actions,
  }: {
    title: string;
    onclose: () => void;
    /** Any CSS length. */
    width?: string;
    /** Without it the content runs edge to edge, for a dialog that lays out its own panes. */
    padded?: boolean;
    children: Snippet;
    /** Buttons in the footer, right-aligned. */
    actions?: Snippet;
  } = $props();

  let sheet: HTMLDialogElement;

  onMount(() => {
    const opener = document.activeElement as HTMLElement | null;
    sheet.showModal();
    sheet.focus();
    return () => opener?.focus();
  });
</script>

<!-- The browser makes the page behind inert, traps Tab and turns Escape into `cancel`. A press
     on the backdrop lands on the dialog element itself, which has no padding. -->
<dialog
  class="surface sheet"
  aria-label={title}
  tabindex="-1"
  bind:this={sheet}
  style:width
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
  onpointerdown={(e) => e.target === e.currentTarget && onclose()}
>
  <header>
    <h2>{title}</h2>
    <IconButton icon="close" label="Close" size={22} iconSize={12} onclick={onclose} />
  </header>
  <div class="content" class:padded>{@render children()}</div>
  {#if actions}<footer>{@render actions()}</footer>{/if}
</dialog>

<style>
  .sheet {
    margin: 12vh auto auto;
    padding: 0;
    color: inherit;
    overflow: visible;
    max-width: calc(100vw - 32px);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
  }
  .sheet::backdrop {
    background: var(--scrim);
  }
  .sheet:focus-visible {
    outline: none;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 36px;
    padding: 0 8px 0 16px;
    flex: none;
    border-bottom: 1px solid var(--line-soft);
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
    font-weight: 600;
  }
  .content {
    overflow: auto;
    min-height: 0;
  }
  .content.padded {
    padding: 16px;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 16px;
    flex: none;
    border-top: 1px solid var(--line-soft);
  }
</style>

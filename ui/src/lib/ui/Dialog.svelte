<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import IconButton from "./IconButton.svelte";

  let {
    title,
    onclose,
    width = "520px",
    children,
    actions,
  }: {
    title: string;
    onclose: () => void;
    /** Any CSS length. */
    width?: string;
    children: Snippet;
    /** Buttons in the footer, right-aligned. */
    actions?: Snippet;
  } = $props();

  let sheet: HTMLElement;

  onMount(() => {
    const opener = document.activeElement as HTMLElement | null;
    sheet.focus();
    return () => opener?.focus();
  });

  /** Modal: Escape closes, and Tab stays inside the sheet. */
  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    } else if (e.key === "Tab") {
      const focusable = [
        ...sheet.querySelectorAll<HTMLElement>("button:enabled, input, select, textarea, [tabindex='0']"),
      ];
      if (!focusable.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const at = document.activeElement;
      if (e.shiftKey && (at === first || at === sheet)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && at === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }
</script>

<!-- The scrim closes the dialog on a press outside the sheet. -->
<div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && onclose()}>
  <div
    class="sheet"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    bind:this={sheet}
    style:width
    {onkeydown}
  >
    <header>
      <h2>{title}</h2>
      <IconButton icon="close" label="Close" size={22} iconSize={12} onclick={onclose} />
    </header>
    <div class="content">{@render children()}</div>
    {#if actions}<footer>{@render actions()}</footer>{/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 12vh;
    background: var(--scrim);
  }
  .sheet {
    max-width: calc(100vw - 32px);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
    background: var(--bg-sheet);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
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
    padding: 16px;
    overflow: auto;
    min-height: 0;
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

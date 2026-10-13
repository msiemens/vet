<script lang="ts">
  // The popup of Search Everywhere and Find in Files: one surface under the header with an optional
  // head row, the query field, the results and a line of key hints. It takes the focus when it opens
  // and gives it back when it closes, unless a click went elsewhere; Escape and a click outside close it.
  import { onDestroy, onMount, type Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import KeyHints from "./KeyHints.svelte";

  let {
    label,
    onclose,
    wide = false,
    value = $bindable(""),
    placeholder,
    invalid = false,
    input = $bindable(),
    body = $bindable(),
    scroll = true,
    onkeydown,
    head,
    trailing,
    children,
    foot,
  }: {
    /** What assistive technology calls the popup and its field. */
    label: string;
    onclose: () => void;
    /** Room for a preview beside the results. */
    wide?: boolean;
    /** The query. */
    value?: string;
    placeholder?: string;
    /** The query is one the search cannot read. */
    invalid?: boolean;
    input?: HTMLInputElement;
    /** The element the results are in. */
    body?: HTMLElement;
    /** The results scroll as a whole; without it they lay out and scroll themselves. */
    scroll?: boolean;
    /** Reads the keys that reach the popup; one it leaves alone, Escape included, is read here. */
    onkeydown?: (e: KeyboardEvent) => void;
    /** A row above the query field. */
    head?: Snippet;
    /** After the query in its row. */
    trailing?: Snippet;
    children: Snippet;
    /** The key hints and what goes beside them. */
    foot: Snippet;
  } = $props();

  let root = $state<HTMLElement>();
  let before: Element | null = null;
  // A click that began before the popup was there is not one outside it: the button that opens it.
  let shownAt = 0;
  // What was clicked outside took the focus itself.
  let clickedAway = false;

  onMount(() => {
    shownAt = performance.now();
    before = document.activeElement;
    input?.focus();
  });
  onDestroy(() => {
    if (!clickedAway && before instanceof HTMLElement && before.isConnected) before.focus({ preventScroll: true });
  });

  function keys(e: KeyboardEvent) {
    onkeydown?.(e);
    if (e.defaultPrevented || e.key !== "Escape" || e.isComposing) return;
    e.preventDefault();
    e.stopPropagation();
    onclose();
  }
</script>

<svelte:window
  onclick={(e) => {
    if (e.timeStamp < shownAt || !root || root.contains(e.target as Node)) return;
    clickedAway = true;
    onclose();
  }}
/>

<div class="surface popup" class:wide role="dialog" aria-label={label} tabindex="-1" bind:this={root} onkeydown={keys}>
  {#if head}<div class="head">{@render head()}</div>{/if}
  <div class="query">
    <Icon name="search" size={14} stroke={1.7} />
    <input
      bind:this={input}
      bind:value
      class:invalid
      type="text"
      spellcheck="false"
      autocomplete="off"
      aria-label={label}
      {placeholder}
    />
    {@render trailing?.()}
  </div>
  <div class="body" class:scroll bind:this={body}>{@render children()}</div>
  <KeyHints>{@render foot()}</KeyHints>
</div>

<style>
  .popup {
    position: fixed;
    z-index: 1000;
    top: calc(var(--h-header) + 6px);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    width: min(var(--w-palette), calc(100vw - 16px));
    max-height: min(var(--h-palette), calc(100vh - var(--h-header) - 40px));
    box-sizing: border-box;
    overflow: hidden;
    font-size: var(--fs);
    color: var(--text);
    background: var(--bg-editor);
  }
  .popup.wide {
    width: min(var(--w-palette-wide), calc(100vw - 16px));
    height: min(var(--h-palette-wide), calc(100vh - var(--h-header) - 40px));
    max-height: none;
  }
  .head {
    flex: none;
    border-bottom: 1px solid var(--line-soft);
  }
  .query {
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--h-palette-field);
    padding: 0 12px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-3);
    flex: none;
  }
  .query input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text);
    font: 14px var(--font-ui);
  }
  .query input.invalid {
    color: var(--del-text);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .body.scroll {
    display: block;
    overflow-y: auto;
    padding-bottom: 6px;
  }
</style>

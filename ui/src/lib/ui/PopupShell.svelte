<script lang="ts">
  // The popup of Search Everywhere, Find in Files, File Structure and the location picker: one surface
  // with an optional head row, the query field (without it the list is typed into, see `SpeedSearch`),
  // the results and an optional line of key hints. It takes the focus when it opens and gives it
  // back when it closes, unless a click went elsewhere or `keepFocus` says what was chosen takes it;
  // Escape and a press outside close it. It sits under the header unless `place` puts it elsewhere.
  import { onDestroy, onMount, type Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import KeyHints from "./KeyHints.svelte";

  let {
    label,
    onclose,
    wide = false,
    place,
    field = true,
    value = $bindable(""),
    placeholder,
    invalid = false,
    input = $bindable(),
    body = $bindable(),
    root = $bindable(),
    keepFocus = $bindable(false),
    scroll = true,
    attributes,
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
    /** The style that places it, where it is not under the header. */
    place?: string;
    /** The query field. */
    field?: boolean;
    /** The query. */
    value?: string;
    placeholder?: string;
    /** The query is one the search cannot read. */
    invalid?: boolean;
    input?: HTMLInputElement;
    /** The element the results are in. */
    body?: HTMLElement;
    root?: HTMLElement;
    /** What is chosen takes the focus itself, so it is not given back. */
    keepFocus?: boolean;
    /** The results scroll as a whole; without it they lay out and scroll themselves. */
    scroll?: boolean;
    /** Attributes of the popup itself. */
    attributes?: Record<string, string | undefined>;
    /** Reads the keys that reach the popup; one it leaves alone, Escape included, is read here. */
    onkeydown?: (e: KeyboardEvent) => void;
    /** A row above the query field. */
    head?: Snippet;
    /** After the query in its row. */
    trailing?: Snippet;
    children: Snippet;
    /** The key hints and what goes beside them. */
    foot?: Snippet;
  } = $props();

  let before: Element | null = null;
  // What was pressed outside took the focus itself.
  let pressedAway = false;

  onMount(() => {
    before = document.activeElement;
    (input ?? root)?.focus();
  });
  onDestroy(() => {
    if (!pressedAway && !keepFocus && before instanceof HTMLElement && before.isConnected) {
      before.focus({ preventScroll: true });
    }
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
  onpointerdown={(e) => {
    if (!root || root.contains(e.target as Node)) return;
    pressedAway = true;
    onclose();
  }}
/>

<div
  class="surface popup"
  class:under={place === undefined}
  class:wide
  role="dialog"
  aria-label={label}
  tabindex="-1"
  style={place}
  bind:this={root}
  onkeydown={keys}
  {...attributes}
>
  {#if head}<div class="head">{@render head()}</div>{/if}
  {#if field}
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
  {/if}
  <div class="body" class:scroll bind:this={body}>{@render children()}</div>
  {#if foot}<KeyHints>{@render foot()}</KeyHints>{/if}
</div>

<style>
  .popup {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    overflow: hidden;
    outline: none;
    font-size: var(--fs);
    color: var(--text);
    background: var(--bg-editor);
  }
  .under {
    top: calc(var(--h-header) + 6px);
    left: 50%;
    transform: translateX(-50%);
    width: min(var(--w-palette), calc(100vw - 16px));
    max-height: min(var(--h-palette), calc(100vh - var(--h-header) - 40px));
  }
  .under.wide {
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
    font: var(--fs-search) var(--font-ui);
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

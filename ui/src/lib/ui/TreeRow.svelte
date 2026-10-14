<script lang="ts">
  // One row of a tree, as the Changes tree draws it: indent by depth, a column for the chevron (empty
  // on a row that holds nothing, so names line up), a glyph, the name, what trails it, a count and a
  // column for the letter of the change. The rows of the file tree, the symbol lists, the groups of
  // Problems, Find and Comments are all this row. A row of the Structure tool is one line; a row of
  // the Symbol panel may `wrap`, so no name is cut.
  import type { Snippet } from "svelte";
  import type { ClassValue } from "svelte/elements";
  import type { StatusLetter } from "../changeLetter";
  import ChangeLetter from "./ChangeLetter.svelte";
  import Chevron from "./Chevron.svelte";

  let {
    depth = 0,
    open,
    tone = "plain",
    mono = false,
    wrap = false,
    gone = false,
    flagged = false,
    heading = false,
    hot = false,
    on = false,
    cur = false,
    interactive = true,
    letters = true,
    chevron = true,
    class: extra,
    title,
    status,
    statusTitle,
    count,
    tabindex,
    disabled = false,
    onclick,
    onmousemove,
    oncontextmenu,
    glyph,
    children,
    trailing,
  }: {
    depth?: number;
    /** A row that holds rows shows a chevron; `undefined` is one that holds none. */
    open?: boolean;
    /** How strongly the row reads: changed code, code one step softer, or only there for context. */
    tone?: "strong" | "plain" | "soft";
    /** The name is code. */
    mono?: boolean;
    wrap?: boolean;
    /** Struck through: a symbol the change removed. */
    gone?: boolean;
    /** A hunk of the file is flagged: its name reads as a warning. */
    flagged?: boolean;
    /** The row names a group of rows, and its name is bold. */
    heading?: boolean;
    /** The group matters more than the others: its count stands out. */
    hot?: boolean;
    /** The selected row. */
    on?: boolean;
    /** The row the keys act on. */
    cur?: boolean;
    /** A button; without it the row is a line of the list that nothing is done with. */
    interactive?: boolean;
    /** The row ends with the column of change letters. */
    letters?: boolean;
    /** The row begins with the chevron column; a row among rows that never fold has none. */
    chevron?: boolean;
    /** Classes of the row's element, for what the list around it marks. */
    class?: ClassValue;
    title?: string;
    /** The letter of the change. */
    status?: StatusLetter;
    statusTitle?: string;
    /** How many rows the row holds or stands for, after what trails the name. */
    count?: number | string;
    tabindex?: number;
    disabled?: boolean;
    onclick?: (e: MouseEvent) => void;
    onmousemove?: (e: MouseEvent) => void;
    oncontextmenu?: (e: MouseEvent) => void;
    glyph?: Snippet;
    children: Snippet;
    trailing?: Snippet;
  } = $props();
</script>

{#snippet content()}
  {#if chevron}<Chevron {open} />{/if}
  {@render glyph?.()}
  <span class="name" class:mono>{@render children()}</span>
  {@render trailing?.()}
  {#if count !== undefined}<span class="n" class:hot>{count}</span>{/if}
  {#if letters}<ChangeLetter letter={status} title={statusTitle} />{/if}
{/snippet}

{#if interactive}
  <button
    class={["flat pick tw", tone, extra]}
    class:wrap
    class:gone
    class:flagged
    class:heading
    class:on
    class:cur
    type="button"
    {tabindex}
    {title}
    {disabled}
    aria-expanded={open}
    style:padding-left="calc(var(--pad-tree) + {depth} * var(--w-indent))"
    {onclick}
    {onmousemove}
    {oncontextmenu}
  >
    {@render content()}
  </button>
{:else}
  <div
    class={["pick tw", tone, extra]}
    class:cur
    {title}
    style:padding-left="calc(var(--pad-tree) + {depth} * var(--w-indent))"
  >
    {@render content()}
  </div>
{/if}

<style>
  .tw {
    display: flex;
    align-items: center;
    gap: var(--gap-tree);
    height: var(--h-row);
    width: 100%;
    padding-right: 10px;
    text-align: left;
    white-space: nowrap;
    font-size: var(--fs-ctl);
    box-sizing: border-box;
  }
  .tw.wrap {
    align-items: flex-start;
    height: auto;
    min-height: var(--h-row);
    padding-block: 2px;
    line-height: 16px;
    white-space: normal;
  }
  .wrap :global(.chev),
  .wrap :global(.change-letter) {
    height: 16px;
    align-items: center;
  }
  .wrap :global(.kind) {
    margin-top: 1px;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wrap .name {
    overflow: visible;
    overflow-wrap: anywhere;
  }
  .name.mono {
    font-size: var(--fs-mono-s);
  }
  .heading .name {
    font-weight: 600;
  }
  .n.hot {
    color: var(--accent);
    font-weight: 600;
  }
  .strong {
    color: var(--text);
  }
  .plain {
    color: var(--text-2);
  }
  .soft {
    color: var(--text-3);
  }
  .soft :global(.kind) {
    opacity: 0.6;
  }
  .flagged .name {
    color: var(--warn);
  }
  .gone .name {
    text-decoration: line-through;
    color: var(--text-2);
  }
  .gone :global(.kind) {
    opacity: 0.6;
  }
</style>

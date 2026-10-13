<script lang="ts">
  // One row of a tree of symbols, as the Changes tree draws it: indent by depth, a column for the
  // chevron (empty on a row that holds nothing, so names line up), a glyph, the name, what trails
  // it, and a column for the letter of the change. A row of the Structure tool is one line; a row
  // of the Symbol panel may `wrap`, so no name is cut.
  import type { Snippet } from "svelte";
  import type { StatusLetter } from "../changeLetter";
  import ChangeLetter from "./ChangeLetter.svelte";
  import Icon from "./Icon.svelte";

  let {
    depth = 0,
    open,
    tone = "plain",
    mono = false,
    wrap = false,
    gone = false,
    on = false,
    cur = false,
    title,
    status,
    statusTitle,
    tabindex,
    disabled = false,
    onclick,
    onmousemove,
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
    /** The selected row. */
    on?: boolean;
    /** The row the keys act on. */
    cur?: boolean;
    title?: string;
    /** The letter of the change. */
    status?: StatusLetter;
    statusTitle?: string;
    tabindex?: number;
    disabled?: boolean;
    onclick?: (e: MouseEvent) => void;
    onmousemove?: (e: MouseEvent) => void;
    glyph?: Snippet;
    children: Snippet;
    trailing?: Snippet;
  } = $props();
</script>

<button
  class="flat tw {tone}"
  class:wrap
  class:gone
  class:on
  class:cur
  type="button"
  {tabindex}
  {title}
  {disabled}
  aria-expanded={open}
  style:padding-left="calc(8px + {depth} * var(--w-indent))"
  {onclick}
  {onmousemove}
>
  <span class="chev"
    >{#if open !== undefined}<Icon name={open ? "open" : "closed"} size={10} />{/if}</span
  >
  {@render glyph?.()}
  <span class="name" class:mono>{@render children()}</span>
  {@render trailing?.()}
  <ChangeLetter letter={status} title={statusTitle} />
</button>

<style>
  .tw {
    display: flex;
    align-items: center;
    gap: 6px;
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
  /* A fixed column, so names line up whether or not a row has a chevron. */
  .chev {
    flex: none;
    width: var(--w-chev);
    display: inline-flex;
    justify-content: center;
  }
  .wrap .chev,
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
  .gone .name {
    text-decoration: line-through;
    color: var(--text-2);
  }
  .gone :global(.kind) {
    opacity: 0.6;
  }
  .on {
    background: var(--toggle);
    color: var(--accent);
  }
  .on .name,
  .on .chev {
    color: inherit;
  }
  .cur {
    background: var(--select);
  }
</style>

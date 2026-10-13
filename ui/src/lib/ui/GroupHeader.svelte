<!-- The line that heads a group of rows in a list, with the count of what is in it. A `group` or a `file`
     folds: a chevron, what it is called, and the count at the end; a `label` only names. A foldable one is a
     row of the list, which the keys and the selection mark like any other. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    kind = "group",
    open,
    count,
    strong = false,
    on = false,
    cur = false,
    indent,
    tip,
    tabindex,
    onclick,
    before,
    children,
    trailing,
  }: {
    /** A group is a heading, a file a name in the list's own text; a label is no row. */
    kind?: "group" | "file" | "label";
    /** Whether what it heads shows; without it a group or file is no row and does not fold. */
    open?: boolean;
    count?: number | string;
    /** The group matters more than the others: its name and count stand out. */
    strong?: boolean;
    /** The selected row. */
    on?: boolean;
    /** The row the keys act on. */
    cur?: boolean;
    /** The left edge of its content, in px. */
    indent?: number;
    tip?: string;
    tabindex?: number;
    onclick?: (e: MouseEvent) => void;
    /** After the chevron. */
    before?: Snippet;
    /** What it is called. */
    children: Snippet;
    /** After the name; for a label, after the count. */
    trailing?: Snippet;
  } = $props();
</script>

{#if kind === "label"}
  <div class="label">
    {@render children()}
    {#if count !== undefined}<span class="n">{count}</span>{/if}
    {@render trailing?.()}
  </div>
{:else if open === undefined}
  <div class="head {kind}" style:padding-left={indent === undefined ? undefined : `${indent}px`}>
    {@render before?.()}
    <span class="text">{@render children()}</span>
    {@render trailing?.()}
    {#if count !== undefined}<span class="n">{count}</span>{/if}
  </div>
{:else}
  <button
    class="flat pick head {kind}"
    class:on
    class:cur
    class:strong
    type="button"
    {tabindex}
    title={tip}
    aria-expanded={open}
    style:padding-left={indent === undefined ? undefined : `${indent}px`}
    {onclick}
  >
    <span class="chev"><Icon name={open ? "open" : "closed"} size={10} /></span>
    {@render before?.()}
    <span class="text">{@render children()}</span>
    {@render trailing?.()}
    {#if count !== undefined}<span class="n" class:strong>{count}</span>{/if}
  </button>
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: var(--h-row);
    box-sizing: border-box;
    padding: 0 12px;
    border-radius: 0;
    text-align: left;
    white-space: nowrap;
    font-size: var(--fs-ctl);
  }
  .group {
    font-weight: 600;
    color: var(--text-2);
  }
  .group.strong,
  .file {
    color: var(--text);
  }
  .chev {
    flex: none;
    width: var(--w-chev);
    display: inline-flex;
    justify-content: center;
    color: var(--text-2);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .n.strong {
    color: var(--accent);
    font-weight: 600;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px 3px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-3);
  }
</style>

<script lang="ts">
  // The rows of a file's symbols, as the Changes tree draws its changed ones: the kind glyph, the
  // name, and the letter of what the change did to it. The window and the popup both show this.
  import { fullName } from "../../lib/symbolLabel";
  import Icon from "../../lib/ui/Icon.svelte";
  import Marked from "../../lib/ui/Marked.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import { keyOf } from "../../state/symbolKey";
  import type { StructureRow } from "../../state/structureRows";
  import { SymbolGlyph } from "../symbols";

  let {
    rows,
    selected,
    cursor = -1,
    list = $bindable(),
    onclick,
    onhover,
  }: {
    rows: readonly StructureRow[];
    /** The symbol selected, by `keyOf`. */
    selected?: string;
    /** The index of the row the keys act on, -1 for none. */
    cursor?: number;
    list?: VirtualRows;
    /** A click on the row at `index`; on a chevron it folds the row instead. */
    onclick: (e: MouseEvent, index: number, chevron: boolean) => void;
    /** The pointer moved over the row at `index`. */
    onhover?: (e: MouseEvent, index: number) => void;
  } = $props();

  const STATUS = { M: "var(--old-base)", A: "var(--add-text)", D: "var(--del-text)" } as const;
  /** The letter a changed symbol ends its row with, as a file does. */
  const LETTER = { modified: "M", added: "A", removed: "D" } as const;
</script>

<VirtualRows bind:this={list} count={rows.length} key={(i) => rows.at(i)?.key ?? ""}>
  {#snippet row(i)}
    {@const r = rows.at(i)}
    {#if r}
      <button
        class="flat tw sym"
        class:ctx={r.muted}
        class:chg={r.change !== undefined}
        class:gone={r.change === "removed"}
        class:on={keyOf(r.symbol.id) === selected}
        class:cur={i === cursor}
        type="button"
        tabindex="-1"
        title={fullName(r.symbol.id)}
        style:padding-left="calc(8px + {r.depth} * var(--w-indent))"
        onclick={(e) => onclick(e, i, e.target instanceof Element && !!e.target.closest(".chev"))}
        onmousemove={(e) => onhover?.(e, i)}
      >
        <span class="chev"
          >{#if r.holds}<Icon name={r.open ? "open" : "closed"} size={10} />{/if}</span
        >
        <SymbolGlyph kind={r.symbol.kind} />
        <span class="name mono"><Marked text={r.name} ranges={r.ranges} /></span>
        <span class="mono status" style:color={r.change && STATUS[LETTER[r.change]]} title={r.change}
          >{r.change ? LETTER[r.change] : ""}</span
        >
      </button>
    {/if}
  {/snippet}
</VirtualRows>

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
  /* A fixed column, so names line up whether or not a row has a chevron. */
  .chev {
    flex: none;
    width: var(--w-chev);
    display: inline-flex;
    justify-content: center;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--fs-mono-s);
  }
  .status {
    flex: none;
    width: 10px;
    font-size: 11px;
    text-align: center;
  }
  /* An unchanged symbol is one step softer than a changed one, so the letters stand out; a row
     shown only to place a match or a change is muted, a removed symbol struck through. */
  .sym {
    color: var(--text-2);
  }
  .sym.chg {
    color: var(--text);
  }
  .sym.ctx {
    color: var(--text-3);
  }
  .sym.ctx :global(.kind) {
    opacity: 0.6;
  }
  .sym.gone .name {
    text-decoration: line-through;
    color: var(--text-2);
  }
  .sym.gone :global(.kind) {
    opacity: 0.6;
  }
  .sym.on {
    background: var(--toggle);
    color: var(--accent);
  }
  .sym.on .name,
  .sym.on .chev {
    color: inherit;
  }
  .cur {
    background: var(--select);
  }
</style>

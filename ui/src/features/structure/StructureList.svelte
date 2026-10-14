<script lang="ts">
  // The rows of a file's symbols, as the Changes tree draws its changed ones: the kind glyph, the
  // name, and the letter of what the change did to it. The window and the popup both show this.
  import { letterOfChange } from "../../lib/changeLetter";
  import { fullName } from "../../lib/symbolLabel";
  import { onChevron } from "../../lib/ui/chevron";
  import Marked from "../../lib/ui/Marked.svelte";
  import TreeRow from "../../lib/ui/TreeRow.svelte";
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
</script>

<VirtualRows bind:this={list} count={rows.length} key={(i) => rows.at(i)?.key ?? ""}>
  {#snippet row(i)}
    {@const r = rows.at(i)}
    {#if r}
      <!-- An unchanged symbol is one step softer than a changed one, so the letters stand out; a row
           shown only to place a match or a change is softer still. -->
      <TreeRow
        depth={r.depth}
        open={r.holds ? r.open : undefined}
        tone={r.muted ? "soft" : r.change !== undefined ? "strong" : "plain"}
        mono
        gone={r.change === "removed"}
        on={keyOf(r.symbol.id) === selected}
        cur={i === cursor}
        title={fullName(r.symbol.id)}
        status={r.change && letterOfChange(r.change)}
        statusTitle={r.change}
        tabindex={-1}
        onclick={(e) => onclick(e, i, onChevron(e))}
        onmousemove={(e) => onhover?.(e, i)}
      >
        {#snippet glyph()}<SymbolGlyph kind={r.symbol.kind} />{/snippet}
        <Marked text={r.name} ranges={r.ranges} />
      </TreeRow>
    {/if}
  {/snippet}
</VirtualRows>

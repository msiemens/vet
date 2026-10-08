<script lang="ts">
  import type { ChangedSymbol } from "../../generated/ChangedSymbol";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import Splitter from "../../lib/ui/Splitter.svelte";
  import { symbols } from "../../state/symbols.svelte";
  import SymbolDetail from "./SymbolDetail.svelte";
  import SymbolList from "./SymbolList.svelte";
  import type { OpenLocation } from "./symbolList";

  /** The panel shows what `symbols` holds; the owner says where a location opens. */
  let { onopen, onreveal }: { onopen: OpenLocation; onreveal: (symbol: ChangedSymbol) => void } = $props();

  const MIN_LIST = 90;
  const MIN_DETAIL = 80;
  let panel = $state<HTMLElement>();
  let top = $state<HTMLElement>();
  let listHeight = $state<number>();
</script>

<aside aria-label="Symbol" bind:this={panel}>
  <div
    class="top"
    bind:this={top}
    style:flex={listHeight === undefined ? undefined : `none`}
    style:height={listHeight === undefined ? undefined : `${listHeight}px`}
  >
    <SymbolList {onreveal} />
  </div>
  <div class="divider">
    <Splitter
      orientation="horizontal"
      label="Resize the symbol list"
      storageKey="vet.symbols.listHeight"
      direction={1}
      current={() => top?.offsetHeight ?? 0}
      limits={() => [MIN_LIST, Math.max(MIN_LIST, (panel?.clientHeight ?? 400) - MIN_DETAIL)]}
      bind:size={listHeight}
      style="top: -2px"
    />
  </div>
  <div class="detail">
    <ScrollArea>
      {#if symbols.selected}
        <SymbolDetail symbol={symbols.selected} {onopen} />
      {:else}
        <p class="empty">No symbol selected</p>
      {/if}
    </ScrollArea>
  </div>
</aside>

<style>
  aside {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .top {
    display: flex;
    flex-direction: column;
    flex: 0 1 40%;
    min-height: 90px;
  }
  /* The list's own bottom border is the line; the splitter straddles it. */
  .divider {
    flex: none;
    position: relative;
    height: 0;
  }
  .detail {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .empty {
    margin: 12px 10px;
    color: var(--text-3);
  }
</style>

<script lang="ts">
  import { symbols } from "../../state/symbols.svelte";
  import SymbolDetail from "./SymbolDetail.svelte";
  import SymbolList from "./SymbolList.svelte";
  import type { OpenLocation } from "./symbolList";

  /** The panel shows what `symbols` holds; the owner says where a location opens. */
  let { onopen }: { onopen: OpenLocation } = $props();
</script>

<aside aria-label="Symbol">
  <div class="top"><SymbolList /></div>
  <div class="detail">
    {#if symbols.selected}
      <SymbolDetail symbol={symbols.selected} {onopen} />
    {:else}
      <p class="empty">No symbol selected</p>
    {/if}
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
  .detail {
    flex: 1;
    min-height: 0;
    overflow: hidden auto;
  }
  .empty {
    margin: 12px 10px;
    color: var(--text-3);
  }
</style>

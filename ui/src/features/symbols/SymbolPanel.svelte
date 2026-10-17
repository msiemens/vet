<script lang="ts">
  // The Symbol tool window: the details of the symbol on show.
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { keyOf } from "../../state/symbolKey";
  import { opened } from "../../state/opened.svelte";
  import SymbolDetail from "./SymbolDetail.svelte";
  import type { OpenLocation } from "./symbolDetail";

  /** The panel shows what the changeset on view's `symbols` holds; the owner says where a location opens. */
  let {
    onopen,
    onfind,
  }: {
    onopen: OpenLocation;
    /** Shows the Find tool, its list focused if `focus`. */
    onfind: (focus: boolean) => void;
  } = $props();
  const symbols = $derived(opened.session?.symbols);
</script>

<aside aria-label="Symbol">
  <div class="sh sh-bar">
    <span>Symbol</span>
  </div>
  <ScrollArea>
    {#if symbols?.selected}
      <!-- A symbol starts with its rows shut as they begin and the lists cut. -->
      {#key `${keyOf(symbols.selected.symbol.id)}${symbols.selected.change}`}
        <SymbolDetail shown={symbols.selected} {onopen} {onfind} />
      {/key}
    {:else if symbols?.unavailable}
      <EmptyState align="start" title={symbols.unavailable.label} text={symbols.unavailable.message} />
    {:else}
      <EmptyState
        align="start"
        title="No symbol selected"
        text="Changed symbols are listed under their files in Changes. Select one there, or put the cursor in a symbol in an editor."
      />
    {/if}
  </ScrollArea>
</aside>

<style>
  aside {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    container-type: inline-size;
  }
</style>

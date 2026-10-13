<script lang="ts">
  // The Symbol tool window: the details of the symbol on show.
  import Icon from "../../lib/ui/Icon.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { keyOf } from "../../state/symbolKey";
  import { symbols } from "../../state/symbols.svelte";
  import SymbolDetail from "./SymbolDetail.svelte";
  import type { OpenLocation } from "./symbolDetail";

  /** The panel shows what `symbols` holds; the owner says where a location opens. */
  let { onopen }: { onopen: OpenLocation } = $props();

  let legend = $state(false);
</script>

<aside aria-label="Symbol">
  <div class="sh sh-bar">
    <span>Symbol</span>
    <span class="grow"></span>
    {#if symbols.selected}
      <button
        class="flat tbtn"
        type="button"
        aria-pressed={legend}
        aria-label="What derived means"
        title="What derived means"
        onclick={() => (legend = !legend)}
      >
        <Icon name="derived" size={14} />
      </button>
    {/if}
  </div>
  {#if legend && symbols.selected}
    <p class="legend">
      <b>Derived.</b> Changed callers, callees, possibly affected, reached from and usages are computed by Vet from the language
      server's answers, not reported by it. The signature is the server's own.
    </p>
  {/if}
  <ScrollArea>
    {#if symbols.selected}
      <!-- A symbol starts with its rows shut as they begin and the lists cut. -->
      {#key `${keyOf(symbols.selected.symbol.id)}${symbols.selected.change}`}
        <SymbolDetail shown={symbols.selected} {onopen} />
      {/key}
    {:else if symbols.unavailable}
      <div class="empty">
        <b>{symbols.unavailable.label}</b>
        <span class="selectable">{symbols.unavailable.message}</span>
      </div>
    {:else}
      <div class="empty">
        <b>No symbol selected</b>
        <span>
          Changed symbols are listed under their files in Changes. Select one there, or put the cursor in a symbol in an
          editor.
        </span>
      </div>
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
  .tbtn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius);
    color: var(--text-3);
  }
  .legend {
    margin: 0;
    padding: 8px 12px;
    background: var(--bg-sheet);
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-2);
    font-size: 12px;
    line-height: 1.45;
  }
  .legend b {
    font-weight: 600;
  }
  .empty {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px 12px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
    line-height: 1.5;
  }
  .empty b {
    color: var(--text-2);
    font-weight: 600;
  }
</style>

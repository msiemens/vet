<script lang="ts">
  import type { FileChange } from "../../generated/FileChange";
  import Tab from "../../lib/ui/Tab.svelte";
  import { lineDelta } from "./editor/lineDelta";
  import { CHANGES, type Tabs, pathOf } from "./Tabs.svelte";

  let { tabs, files }: { tabs: Tabs; files: FileChange[] } = $props();

  const byPath = $derived(new Map(files.map((f) => [f.path, f])));
</script>

{#snippet delta(of: readonly FileChange[])}
  {@const { added, removed } = lineDelta(of)}
  <span class="mono delta"><span class="add">+{added}</span> <span class="del">−{removed}</span></span>
{/snippet}

<div class="tabs" role="tablist" aria-label="Open views">
  {#each tabs.open as id (id)}
    {@const path = pathOf(id)}
    {@const file = path === undefined ? undefined : byPath.get(path)}
    <Tab
      selected={tabs.active === id}
      title={path ?? "Every changed file"}
      onselect={() => (tabs.active = id)}
      onclose={id === CHANGES ? undefined : () => tabs.close(id)}
    >
      {#if path === undefined}
        Changes {@render delta(files)}
      {:else}
        {path.slice(path.lastIndexOf("/") + 1)}
        {#if file}{@render delta([file])}{/if}
      {/if}
    </Tab>
  {/each}
</div>

<style>
  .tabs {
    display: flex;
    align-items: flex-end;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .delta {
    font-size: 11px;
  }
  .add {
    color: var(--addt);
  }
  .del {
    color: var(--delt);
  }
</style>

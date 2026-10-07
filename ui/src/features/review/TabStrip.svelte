<script lang="ts">
  import type { FileChange } from "../../generated/FileChange";
  import type { LineDelta } from "../../generated/LineDelta";
  import { basename } from "../../lib/paths";
  import Tab from "../../lib/ui/Tab.svelte";
  import { CHANGES, type Tabs, pathOf } from "./Tabs.svelte";

  /** `delta` is the changeset's. */
  let { tabs, files, delta }: { tabs: Tabs; files: FileChange[]; delta: LineDelta } = $props();

  const byPath = $derived(new Map(files.map((f) => [f.path, f])));
</script>

{#snippet counts({ added, removed }: LineDelta)}
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
        Changes {@render counts(delta)}
      {:else}
        {basename(path)}
        {#if file}{@render counts(file.delta)}{/if}
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
    color: var(--add-text);
  }
  .del {
    color: var(--del-text);
  }
</style>

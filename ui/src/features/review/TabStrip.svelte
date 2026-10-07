<script lang="ts">
  import type { FileChange } from "../../generated/FileChange";
  import { basename } from "../../lib/paths";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Tab from "../../lib/ui/Tab.svelte";
  import { CHANGES, type TabId, type Tabs, pathOf } from "./Tabs.svelte";

  /** `files` are the ones the first tab lists; `onmenu` opens the context menu of a tab. */
  let { tabs, files, onmenu }: { tabs: Tabs; files: FileChange[]; onmenu: (id: TabId, x: number, y: number) => void } =
    $props();
</script>

<div class="tabs" role="tablist" aria-label="Open views">
  {#each tabs.open as id (id)}
    {@const path = pathOf(id)}
    <Tab
      selected={tabs.active === id}
      title={path ?? "Every changed file"}
      onselect={() => (tabs.active = id)}
      onclose={id === CHANGES ? undefined : () => tabs.close(id)}
      onmenu={(x, y) => onmenu(id, x, y)}
    >
      {#if path === undefined}
        All changes <span class="count">{files.length}</span>
      {:else}
        <FileIcon name={basename(path)} />
        {basename(path)}
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
  .count {
    margin-left: 4px;
    color: var(--text-3);
  }
</style>

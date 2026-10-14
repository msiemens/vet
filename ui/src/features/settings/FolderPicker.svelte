<!-- The folders of the repository as a tree, to pick one. -->
<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import TreeRow from "../../lib/ui/TreeRow.svelte";
  import { changeset } from "../../state/changeset.svelte";
  import { type FolderRow, folderRows } from "./folderRows";

  let { onpick, oncancel }: { onpick: (path: string) => void; oncancel: () => void } = $props();

  const listed = new SvelteMap<string, readonly string[]>();
  const open = new SvelteSet<string>();
  let chosen = $state<string>();
  const rows = $derived(folderRows(listed, open));

  async function list(path: string) {
    if (listed.has(path)) return;
    const names = await changeset.folders(path);
    if (names) listed.set(path, names);
  }

  function click(row: FolderRow) {
    chosen = row.path;
    if (open.delete(row.path)) return;
    open.add(row.path);
    void list(row.path);
  }

  onMount(() => void list(""));
</script>

<div class="picker">
  <div class="tree">
    <ScrollArea>
      {#each rows as row (row.path)}
        <TreeRow depth={row.depth} open={row.open} letters={false} on={row.path === chosen} onclick={() => click(row)}>
          {#snippet glyph()}<FileIcon name={row.name} folder open={row.open} />{/snippet}
          {row.name}
        </TreeRow>
      {/each}
    </ScrollArea>
  </div>
  <div class="foot">
    <span class="path grow">{chosen ?? ""}</span>
    <button class="ctl" type="button" onclick={oncancel}>Cancel</button>
    <button
      class="ctl primary"
      type="button"
      disabled={chosen === undefined}
      onclick={() => chosen !== undefined && onpick(chosen)}>Add folder</button
    >
  </div>
</div>

<style>
  .picker {
    border-top: 1px solid var(--line);
    background: var(--bg-sheet);
  }
  .tree {
    max-height: 230px;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-top: 1px solid var(--line-soft);
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
    color: var(--text-2);
  }
</style>

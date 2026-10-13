<script lang="ts">
  // What the window and the popup say instead of a list: no file, the outline on its way, why there
  // is none, or that the filters left nothing.
  import type { OutlineLoad } from "../../state/structure.svelte";

  let {
    load,
    file,
    matches,
  }: {
    load: OutlineLoad;
    /** The name of the file; none when the active tab shows no file. */
    file?: string;
    /** How many rows the filters left. */
    matches: number;
  } = $props();

  const outline = $derived(load.state === "done" ? load.outline : undefined);
  const total = $derived(outline?.symbols.length ?? 0);
</script>

{#if file === undefined}
  <div class="empty"><b>No file</b><span>Open a file to see its symbols.</span></div>
{:else if load.state === "loading"}
  <div class="empty"><span>Loading the symbols of {file}…</span></div>
{:else if load.state === "failed"}
  <div class="empty"><b>Symbols not found</b><span class="err">{load.message}</span></div>
{:else if outline?.unavailable}
  <div class="empty">
    <b>{outline.unavailable.label}</b>
    <span>{outline.unavailable.message}</span>
  </div>
{:else if !total}
  <div class="empty"><b>No symbols</b><span>{file} declares none.</span></div>
{:else if !matches}
  <div class="empty"><b>No matches</b><span>No symbol of {file} matches.</span></div>
{/if}

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
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

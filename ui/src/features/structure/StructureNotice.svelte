<script lang="ts">
  // What the window and the popup say instead of a list: no file, the outline on its way, why there
  // is none, or that the filters left nothing.
  import EmptyState from "../../lib/ui/EmptyState.svelte";
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
  <EmptyState align="start" title="No file" text="Open a file to see its symbols." />
{:else if load.state === "loading"}
  <EmptyState align="start" text="Loading the symbols of {file}…" />
{:else if load.state === "failed"}
  <EmptyState align="start" title="Symbols not found" text={load.message} error />
{:else if outline?.unavailable}
  <EmptyState align="start" title={outline.unavailable.label} text={outline.unavailable.message} />
{:else if !total}
  <EmptyState align="start" title="No symbols" text="{file} declares none." />
{:else if !matches}
  <EmptyState align="start" title="No matches" text="No symbol of {file} matches." />
{/if}

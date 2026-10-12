<script lang="ts">
  // Where a thread is, for a row: its file path with the line after the name, or `changeset` for the changeset's own.
  import type { Thread } from "../../generated/Thread";
  import PathText from "../../lib/ui/PathText.svelte";
  import { threadPlace } from "./groups";

  let { thread, changeset = "Changeset" }: { thread: Thread; changeset?: string } = $props();

  const place = $derived(threadPlace(thread));
</script>

{#if place}<PathText path={place.path} suffix={place.suffix} />{:else}<span class="path-text">{changeset}</span>{/if}

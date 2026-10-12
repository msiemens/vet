<script lang="ts">
  // A qualified name with its qualifier dimmed and cut first, and its last segment whole.
  import { splitQualified } from "../qualified";
  import type { Range } from "../speedSearch";
  import SplitText from "./SplitText.svelte";

  let { name, title = name, ranges }: { name: string; title?: string; ranges?: readonly Range[] } = $props();

  const parts = $derived(splitQualified(name));
</script>

<SplitText lead={parts.qualifier} last={parts.name} {title} {ranges} />

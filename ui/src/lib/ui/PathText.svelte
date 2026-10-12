<script lang="ts">
  // A path with its folders dimmed and cut first, and its file name whole. `suffix` (`:12`) belongs to the name.
  import { splitPath } from "../paths";
  import type { Range } from "../speedSearch";
  import SplitText from "./SplitText.svelte";

  let {
    path,
    suffix = "",
    title = path,
    ranges,
  }: { path: string; suffix?: string; title?: string; ranges?: readonly Range[] } = $props();

  const parts = $derived(splitPath(path));
</script>

<SplitText lead={parts.dir} last={parts.name + suffix} {title} {ranges} />

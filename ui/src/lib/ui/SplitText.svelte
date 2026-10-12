<script lang="ts">
  // A path or qualified name in two parts on one line: the leading part dimmed and cut first, the
  // last segment whole (`.path-text`). Use `PathText` or `QualifiedName`, which do the splitting.
  import { sliceRanges, type Range } from "../speedSearch";
  import Marked from "./Marked.svelte";

  let {
    lead,
    last,
    title,
    ranges,
  }: {
    lead: string;
    last: string;
    /** The full text, where the line is cut. */
    title?: string;
    /** Search matches in `lead + last`. */
    ranges?: readonly Range[];
  } = $props();
</script>

<span class="path-text" {title}
  ><span class="dir"><Marked text={lead} ranges={ranges && sliceRanges(ranges, 0, lead.length)} /></span><span
    class="name"
    ><Marked text={last} ranges={ranges && sliceRanges(ranges, lead.length, lead.length + last.length)} /></span
  ></span
>

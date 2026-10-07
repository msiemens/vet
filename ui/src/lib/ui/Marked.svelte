<script lang="ts">
  import { highlightRuns, type Range } from "../speedSearch";

  /** `text` with the characters in `ranges` (merged, sorted) marked as search matches. */
  let { text, ranges }: { text: string; ranges?: readonly Range[] } = $props();
</script>

{#if ranges?.length}{#each highlightRuns(text, ranges) as run, i (i)}{#if run.hit}<mark>{run.text}</mark
      >{:else}{run.text}{/if}{/each}{:else}{text}{/if}

<style>
  mark {
    background: var(--select);
    color: inherit;
    border-radius: 2px;
  }
</style>

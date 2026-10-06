<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    mono = false,
    bold = false,
    onclick,
    title,
    children,
  }: {
    mono?: boolean;
    bold?: boolean;
    onclick?: (e: MouseEvent) => void;
    title?: string;
    children: Snippet;
  } = $props();
</script>

<button class="tool" class:mono class:bold type="button" {title} {onclick}>{@render children()}</button>

<style>
  .mono {
    font-size: var(--fs-mono);
  }
  .bold {
    font-weight: 600;
  }
</style>

<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    mono = false,
    bold = false,
    active,
    onclick,
    title,
    children,
  }: {
    mono?: boolean;
    bold?: boolean;
    /** Makes it a toggle, e.g. while its menu is open; `undefined` is a plain action. */
    active?: boolean;
    onclick?: (e: MouseEvent) => void;
    title?: string;
    children: Snippet;
  } = $props();
</script>

<button class="tool" class:mono class:bold type="button" aria-pressed={active} {title} {onclick}
  >{@render children()}</button
>

<style>
  .mono {
    font-size: var(--fs-mono);
  }
  .bold {
    font-weight: 600;
  }
</style>

<script lang="ts">
  // An on/off filter, as a pill: on, it narrows what a list shows.
  import type { Snippet } from "svelte";

  let {
    on,
    disabled,
    focusable = true,
    title,
    onclick,
    children,
  }: {
    on: boolean;
    disabled?: boolean;
    /** Out of the tab order where the keys belong to a field beside it. */
    focusable?: boolean;
    title?: string;
    onclick: () => void;
    children: Snippet;
  } = $props();
</script>

<button
  class="ctl chip"
  type="button"
  tabindex={focusable ? undefined : -1}
  aria-pressed={on}
  {disabled}
  {title}
  {onclick}>{@render children()}</button
>

<style>
  .chip {
    height: var(--h-ctl-s);
    padding: 0 9px;
    border-radius: calc(var(--h-ctl-s) / 2);
    font-size: var(--fs-mono-s);
  }
</style>

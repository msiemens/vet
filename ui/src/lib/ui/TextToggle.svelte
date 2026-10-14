<script lang="ts">
  // A toggle in a panel's header that is a few letters, where an icon would not say it.
  import type { Snippet } from "svelte";

  let {
    on,
    disabled,
    title,
    onclick,
    children,
  }: {
    on: boolean;
    disabled?: boolean;
    title: string;
    onclick: () => void;
    children: Snippet;
  } = $props();
</script>

<button class="flat toggle mono" type="button" aria-pressed={on} {disabled} {title} {onclick}
  >{@render children()}</button
>

<style>
  .toggle {
    height: var(--h-ctl-s);
    flex: none;
    padding: 0 6px;
    border-radius: var(--radius);
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: 500;
    line-height: 1;
  }
</style>

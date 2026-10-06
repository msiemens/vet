<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    icon,
    label,
    size = 26,
    iconSize = 16,
    active,
    onclick,
  }: {
    icon: IconName;
    label: string;
    size?: number;
    iconSize?: number;
    /** Makes it a toggle button; `undefined` is a plain action. */
    active?: boolean;
    onclick?: (e: MouseEvent) => void;
  } = $props();
</script>

<button
  class="flat ib"
  type="button"
  aria-label={label}
  title={label}
  aria-pressed={active}
  style:width="{size}px"
  style:height="{size}px"
  {onclick}
>
  <Icon name={icon} size={iconSize} />
</button>

<style>
  .ib {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    color: var(--tx2);
    flex: none;
  }
</style>

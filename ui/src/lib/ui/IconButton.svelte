<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    icon,
    label,
    size = 26,
    iconSize = 16,
    active = false,
    onclick,
  }: {
    icon: IconName;
    label: string;
    size?: number;
    iconSize?: number;
    active?: boolean;
    onclick?: (e: MouseEvent) => void;
  } = $props();
</script>

<button
  class="ib"
  class:active
  type="button"
  aria-label={label}
  title={label}
  aria-pressed={active || undefined}
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
    border: 0;
    background: transparent;
    border-radius: var(--radius);
    color: var(--tx2);
    padding: 0;
    cursor: pointer;
    flex: none;
  }
  .ib:hover {
    background: var(--hov);
  }
  .ib.active {
    background: var(--sel);
    color: var(--acc);
  }
</style>

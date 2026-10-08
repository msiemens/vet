<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    icon,
    label,
    size = 26,
    iconSize = 16,
    title = label,
    disabled,
    active,
    button = $bindable(),
    onclick,
  }: {
    icon: IconName;
    label: string;
    /** The tooltip; the label by default. */
    title?: string;
    disabled?: boolean;
    size?: number;
    iconSize?: number;
    /** Makes it a toggle button; `undefined` is a plain action. */
    active?: boolean;
    /** The element, for focusing it. */
    button?: HTMLButtonElement;
    onclick?: (e: MouseEvent) => void;
  } = $props();
</script>

<button
  bind:this={button}
  class="flat ib"
  type="button"
  aria-label={label}
  {title}
  {disabled}
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
    color: var(--text-2);
    flex: none;
  }
</style>

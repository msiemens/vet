<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    icon,
    label,
    size,
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
    /** The height of a control of that size (`--h-ctl-m`, `-s`, `-xs`, `-xxs`); a control's own by default. */
    size?: "m" | "s" | "xs" | "xxs";
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
  class={["flat ib", size]}
  type="button"
  aria-label={label}
  {title}
  {disabled}
  aria-pressed={active}
  {onclick}
>
  <Icon name={icon} size={iconSize} />
</button>

<style>
  .ib {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--h-ctl);
    height: var(--h-ctl);
    border-radius: var(--radius);
    color: var(--text-2);
    flex: none;
  }
  .m {
    width: var(--h-ctl-m);
    height: var(--h-ctl-m);
  }
  .s {
    width: var(--h-ctl-s);
    height: var(--h-ctl-s);
  }
  .xs {
    width: var(--h-ctl-xs);
    height: var(--h-ctl-xs);
  }
  .xxs {
    width: var(--h-ctl-xxs);
    height: var(--h-ctl-xxs);
  }
</style>

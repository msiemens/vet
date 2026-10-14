<script lang="ts" generics="T extends string">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  let {
    options,
    value,
    onchange,
    label,
    small = false,
  }: {
    options: { id: T; label: string; icon?: IconName; title?: string; disabled?: boolean }[];
    value: T;
    onchange: (id: T) => void;
    /** Names the group for assistive technology. */
    label: string;
    /** The height of toolbar controls instead of form controls. */
    small?: boolean;
  } = $props();
</script>

<div class="ctl-group" class:small role="group" aria-label={label}>
  {#each options as option (option.id)}
    <button
      class="ctl"
      type="button"
      title={option.title}
      disabled={option.disabled}
      aria-pressed={option.id === value}
      onclick={() => onchange(option.id)}
      >{#if option.icon}<Icon name={option.icon} size={13} />{/if}{option.label}</button
    >
  {/each}
</div>

<style>
  .ctl {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .small .ctl {
    height: var(--h-ctl-s);
    font-size: var(--fs-mono);
  }
</style>

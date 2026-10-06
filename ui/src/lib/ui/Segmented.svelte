<script lang="ts" generics="T extends string">
  let {
    options,
    value,
    onchange,
    label,
    small = false,
  }: {
    options: { id: T; label: string }[];
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
    <button class="ctl" type="button" aria-pressed={option.id === value} onclick={() => onchange(option.id)}
      >{option.label}</button
    >
  {/each}
</div>

<style>
  .small .ctl {
    height: 22px;
    font-size: 12px;
  }
</style>

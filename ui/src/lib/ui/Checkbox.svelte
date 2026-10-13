<!-- A checkbox with its label, which is `children`. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    checked,
    disabled = false,
    onchange,
    children,
  }: {
    checked: boolean;
    disabled?: boolean;
    onchange: (checked: boolean) => void;
    children: Snippet;
  } = $props();
</script>

<label>
  <input type="checkbox" {checked} {disabled} onchange={(e) => onchange(e.currentTarget.checked)} />
  {@render children()}
</label>

<style>
  label {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-size: var(--fs-ctl);
  }
</style>

<script lang="ts">
  // The small filter field of a panel's toolbar: a magnifier inside a one-line input.
  import Icon from "./Icon.svelte";

  let {
    value = $bindable(""),
    label,
    placeholder = "Filter",
    input = $bindable(),
    onkeydown,
  }: {
    value?: string;
    /** What assistive technology calls the field. */
    label: string;
    placeholder?: string;
    /** The input element, for the owner to move the focus to. */
    input?: HTMLInputElement;
    onkeydown?: (e: KeyboardEvent) => void;
  } = $props();
</script>

<label class="find">
  <Icon name="search" size={13} />
  <input bind:this={input} bind:value type="text" {placeholder} aria-label={label} spellcheck="false" {onkeydown} />
</label>

<style>
  .find {
    position: relative;
    flex: 0 1 150px;
    min-width: 80px;
    display: inline-flex;
    align-items: center;
    color: var(--text-3);
  }
  .find :global(svg) {
    position: absolute;
    left: 6px;
    pointer-events: none;
  }
  input {
    height: 22px;
    width: 100%;
    min-width: 0;
    box-sizing: border-box;
    padding: 0 6px 0 24px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-control);
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
</style>

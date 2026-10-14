<script lang="ts">
  // A bordered one-line field with a magnifier inside, to search or filter what is listed. `small`
  // is the size for a panel's toolbar.
  import Icon from "./Icon.svelte";

  let {
    value = $bindable(""),
    label,
    placeholder,
    input = $bindable(),
    small = false,
    width,
    autofocus = false,
    onkeydown,
    oninput,
  }: {
    value?: string;
    /** What assistive technology calls the field. */
    label: string;
    placeholder?: string;
    /** The input element, for the owner to move the focus to. */
    input?: HTMLInputElement;
    small?: boolean;
    /** The width it asks for in px; without it a small field is 150 and the others fill their row. */
    width?: number;
    /** Takes the focus when it appears. */
    autofocus?: boolean;
    onkeydown?: (e: KeyboardEvent) => void;
    oninput?: (e: Event) => void;
  } = $props();

  const basis = $derived(width ?? (small ? 150 : undefined));
</script>

<label class="search" class:small style:flex={basis === undefined ? undefined : `0 1 ${basis}px`}>
  <Icon name="search" size={13} />
  <!-- svelte-ignore a11y_autofocus -->
  <input
    bind:this={input}
    bind:value
    class="field"
    type="text"
    {placeholder}
    aria-label={label}
    spellcheck="false"
    autocomplete="off"
    {autofocus}
    {onkeydown}
    {oninput}
  />
</label>

<style>
  .search {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    color: var(--text-3);
  }
  .search :global(svg) {
    position: absolute;
    left: 8px;
    pointer-events: none;
  }
  .search input {
    flex: 1;
    padding-left: 26px;
  }
  .small {
    min-width: 80px;
  }
  .small :global(svg) {
    left: 6px;
  }
  .small input {
    height: var(--h-ctl-s);
    padding: 0 6px 0 24px;
    background: var(--bg-control);
    font-size: var(--fs-mono);
  }
</style>

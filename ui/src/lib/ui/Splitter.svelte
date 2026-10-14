<script lang="ts">
  // A draggable divider that sizes the pane next to it: by pointer, by arrow keys, back to the
  // default on a double click. The size is remembered as the preference `name`.
  import { numberPreference } from "../preferences.svelte";

  let {
    orientation,
    label,
    name,
    direction,
    current,
    limits,
    size = $bindable(),
    style,
  }: {
    /** "vertical" divides left from right (it moves along x), "horizontal" top from bottom. */
    orientation: "vertical" | "horizontal";
    label: string;
    name: string;
    /** +1 when moving right/down grows the pane, -1 when it shrinks it. */
    direction: 1 | -1;
    /** The pane's size in px as laid out now. */
    current: () => number;
    /** The smallest and largest size in px the pane may take, as laid out now. */
    limits: () => [number, number];
    /** The size the viewer set, in px; undefined while the pane keeps its default. */
    size?: number;
    /** Where the divider sits (the owner positions it). */
    style?: string;
  } = $props();

  const STEP = 16;
  const horizontal = $derived(orientation === "horizontal");

  // The name does not change for a mounted splitter.
  // svelte-ignore state_referenced_locally
  const kept = numberPreference("app", `splitter.${name}`);
  if (size === undefined && kept.value !== undefined && kept.value > 0) size = kept.value;

  let drag: { at: number; size: number } | undefined;
  let dragging = $state(false);

  function resize(next: number) {
    const [min, max] = limits();
    size = Math.min(max, Math.max(min, next));
  }

  const coordinate = (e: PointerEvent) => (horizontal ? e.clientY : e.clientX);

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { at: coordinate(e), size: current() };
    dragging = true;
  }

  function onpointermove(e: PointerEvent) {
    if (drag) resize(drag.size + direction * (coordinate(e) - drag.at));
  }

  function onpointerup() {
    if (!drag) return;
    drag = undefined;
    dragging = false;
    kept.value = size;
  }

  function onkeydown(e: KeyboardEvent) {
    const [less, more] = horizontal ? ["ArrowUp", "ArrowDown"] : ["ArrowLeft", "ArrowRight"];
    const sign = e.key === more ? 1 : e.key === less ? -1 : 0;
    if (!sign) return;
    e.preventDefault();
    resize(current() + sign * direction * STEP);
    kept.value = size;
  }

  function reset() {
    size = undefined;
    kept.value = undefined;
  }
</script>

<!-- A focusable separator is a widget (WAI-ARIA window splitter). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="splitter {orientation}"
  class:dragging
  role="separator"
  aria-orientation={orientation}
  aria-label={label}
  aria-valuenow={size}
  tabindex="0"
  {style}
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  {onkeydown}
  ondblclick={reset}
></div>

<style>
  .splitter {
    position: absolute;
    z-index: 10;
  }
  .vertical {
    top: 0;
    bottom: 0;
    width: var(--w-splitter);
    cursor: col-resize;
  }
  .horizontal {
    left: 0;
    right: 0;
    height: var(--w-splitter);
    cursor: row-resize;
  }
  .splitter:hover {
    background: var(--toggle-press);
  }
  .splitter.dragging {
    background: var(--focus);
  }
</style>

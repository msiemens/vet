<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    side,
    defaultWidth,
    children,
  }: {
    side: "left" | "right";
    /** Any CSS length, normally a `--w-*` token; the width until the viewer drags the splitter. */
    defaultWidth: string;
    children: Snippet;
  } = $props();

  const STEP = 16;
  // Each side keeps its own width.
  const storageKey = (s: string) => `vet.sidebar.${s}`;

  function storedWidth(s: string): number | undefined {
    try {
      const width = Number(localStorage.getItem(storageKey(s)));
      return width > 0 ? width : undefined;
    } catch {
      return undefined; // Storage may be unavailable; the width then lasts for this session only.
    }
  }

  function store(s: string, width: number | undefined) {
    try {
      if (width === undefined) localStorage.removeItem(storageKey(s));
      else localStorage.setItem(storageKey(s), String(width));
    } catch {
      // See `storedWidth`.
    }
  }

  let panel: HTMLElement;
  // `side` does not change for a mounted bar.
  // svelte-ignore state_referenced_locally
  let width = $state(storedWidth(side));
  let drag: { x: number; width: number } | undefined;
  let dragging = $state(false);

  /** The token-defined limits, as the browser resolves them. */
  function limits(): [number, number] {
    const style = getComputedStyle(panel);
    return [parseFloat(style.minWidth), parseFloat(style.maxWidth)];
  }

  function resize(next: number) {
    const [min, max] = limits();
    width = Math.min(max, Math.max(min, next));
  }

  const direction = $derived(side === "left" ? 1 : -1);

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, width: panel.offsetWidth };
    dragging = true;
  }

  function onpointermove(e: PointerEvent) {
    if (drag) resize(drag.width + direction * (e.clientX - drag.x));
  }

  function onpointerup() {
    if (!drag) return;
    drag = undefined;
    dragging = false;
    store(side, width);
  }

  function onkeydown(e: KeyboardEvent) {
    const sign = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!sign) return;
    e.preventDefault();
    resize(panel.offsetWidth + sign * direction * STEP);
    store(side, width);
  }

  function reset() {
    width = undefined;
    store(side, undefined);
  }
</script>

<aside class={side} bind:this={panel} style:width={width === undefined ? defaultWidth : `${width}px`}>
  {@render children()}
  <!-- A focusable separator is a widget (WAI-ARIA window splitter). -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="splitter"
    class:dragging
    role="separator"
    aria-orientation="vertical"
    aria-label="Resize sidebar"
    aria-valuenow={width}
    tabindex="0"
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    {onkeydown}
    ondblclick={reset}
  ></div>
</aside>

<style>
  aside {
    position: relative;
    flex: none;
    min-width: var(--w-side-min);
    max-width: var(--w-side-max);
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--panel);
  }
  .left {
    border-right: 1px solid var(--ln);
  }
  .right {
    border-left: 1px solid var(--ln);
  }
  .splitter {
    position: absolute;
    top: 0;
    bottom: 0;
    z-index: 10;
    width: var(--w-splitter);
    cursor: col-resize;
  }
  .left .splitter {
    right: calc(var(--w-splitter) / -2);
  }
  .right .splitter {
    left: calc(var(--w-splitter) / -2);
  }
  .splitter:hover {
    background: var(--tog-press);
  }
  .splitter.dragging {
    background: var(--focus);
  }
</style>

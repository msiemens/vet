<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import { readJson, writeStored } from "../storage";
  import IconButton from "./IconButton.svelte";
  import ScrollArea from "./ScrollArea.svelte";

  let {
    title,
    onclose,
    width = "520px",
    height,
    resizable,
    padded = true,
    children,
    actions,
    bar,
    closeButton = true,
  }: {
    title: string;
    onclose: () => void;
    /** Any CSS length; the width until the viewer resizes a `resizable` dialog. */
    width?: string;
    /** Any CSS length; without it the dialog is as tall as its content, up to the window. */
    height?: string;
    /** Lets the viewer drag the dialog's edges; the size is kept under this name. */
    resizable?: string;
    /** Without it the content runs edge to edge and does not scroll, for a dialog that lays out its own panes. */
    padded?: boolean;
    children: Snippet;
    /** Buttons in the footer, right-aligned. */
    actions?: Snippet;
    /** Controls in the title row, right-aligned before the close button. */
    bar?: Snippet;
    /** Whether the title row has a close button; without it Escape and a press outside close the dialog. */
    closeButton?: boolean;
  } = $props();

  const MIN = { w: 360, h: 240 };
  // The dialog sits at a fixed distance from the top and is centred sideways.
  const TOP = 0.12;
  const storageKey = (name: string) => `vet.dialog.${name}`;

  interface Size {
    w: number;
    h: number;
  }

  function storedSize(name: string | undefined): Size | undefined {
    if (!name) return undefined;
    return readJson(storageKey(name), (value) => {
      const { w, h } = (value ?? {}) as Partial<Size>;
      return typeof w === "number" && typeof h === "number" && w >= MIN.w && h >= MIN.h ? { w, h } : undefined;
    });
  }

  let sheet: HTMLDialogElement;
  // `resizable` does not change for a mounted dialog.
  // svelte-ignore state_referenced_locally
  let size = $state(storedSize(resizable));
  let drag: { x: number; y: number; start: Size; dx: -1 | 0 | 1; dy: 0 | 1 } | undefined;

  const EDGES = [
    { name: "left", dx: -1, dy: 0 },
    { name: "right", dx: 1, dy: 0 },
    { name: "bottom", dx: 0, dy: 1 },
    { name: "bottom-left", dx: -1, dy: 1 },
    { name: "bottom-right", dx: 1, dy: 1 },
  ] as const;

  function onpointerdown(e: PointerEvent, dx: -1 | 0 | 1, dy: 0 | 1) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, start: { w: sheet.offsetWidth, h: sheet.offsetHeight }, dx, dy };
  }

  /** Sets the size, kept within the window and above the smallest. */
  function resize(w: number, h: number) {
    size = {
      w: Math.min(innerWidth - 32, Math.max(MIN.w, w)),
      h: Math.min(innerHeight * (1 - TOP) - 16, Math.max(MIN.h, h)),
    };
  }

  function keep() {
    if (resizable) writeStored(storageKey(resizable), JSON.stringify(size));
  }

  function onpointermove(e: PointerEvent) {
    if (!drag) return;
    // Centred, so a side moves by half of what the width changes by.
    const w = drag.start.w + 2 * drag.dx * (e.clientX - drag.x);
    const h = drag.start.h + drag.dy * (e.clientY - drag.y);
    resize(drag.dx ? w : drag.start.w, h);
  }

  function onpointerup() {
    if (!drag) return;
    drag = undefined;
    keep();
  }

  const STEP = 16;

  /** The right edge widens with the right arrow, the bottom one lengthens with the down arrow; Shift steps further. */
  function onkeydown(e: KeyboardEvent, edge: "right" | "bottom") {
    const [less, more] = edge === "right" ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
    const sign = e.key === more ? 1 : e.key === less ? -1 : 0;
    if (!sign) return;
    e.preventDefault();
    const step = sign * STEP * (e.shiftKey ? 4 : 1);
    const [w, h] = [sheet.offsetWidth, sheet.offsetHeight];
    // Centred, so the width grows on both sides: twice the step moves the edge by one.
    if (edge === "right") resize(w + 2 * step, h);
    else resize(w, h + step);
    keep();
  }

  onMount(() => {
    const opener = document.activeElement as HTMLElement | null;
    sheet.showModal();
    sheet.focus();
    return () => opener?.focus();
  });
</script>

<!-- The browser makes the page behind inert, traps Tab and turns Escape into `cancel`. A press
     on the backdrop lands on the dialog element itself, which has no padding. -->
<dialog
  class="surface sheet"
  class:resizable={!!resizable}
  aria-label={title}
  tabindex="-1"
  bind:this={sheet}
  style:width={size ? `${size.w}px` : width}
  style:height={size ? `${size.h}px` : height}
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
  onpointerdown={(e) => e.target === e.currentTarget && onclose()}
>
  <header>
    <h2>{title}</h2>
    {#if bar}<div class="bar">{@render bar()}</div>{/if}
    {#if closeButton}<IconButton icon="close" label="Close" size="s" iconSize={12} onclick={onclose} />{/if}
  </header>
  {#if padded}
    <ScrollArea><div class="padded">{@render children()}</div></ScrollArea>
  {:else}
    <div class="content">{@render children()}</div>
  {/if}
  {#if actions}<footer>{@render actions()}</footer>{/if}
  {#if resizable}
    {#each EDGES as edge (edge.name)}
      {#if edge.name === "right" || edge.name === "bottom"}
        {@const name = edge.name}
        <!-- The right and bottom edges are also the keyboard's way to resize: focusable window splitters. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div
          class="edge {name}"
          role="separator"
          aria-orientation={name === "right" ? "vertical" : "horizontal"}
          aria-label={name === "right" ? "Dialog width" : "Dialog height"}
          aria-valuenow={size && (name === "right" ? size.w : size.h)}
          tabindex="0"
          onpointerdown={(e) => onpointerdown(e, edge.dx, edge.dy)}
          {onpointermove}
          {onpointerup}
          onkeydown={(e) => onkeydown(e, name)}
        ></div>
      {:else}
        <div
          class="edge {edge.name}"
          aria-hidden="true"
          onpointerdown={(e) => onpointerdown(e, edge.dx, edge.dy)}
          {onpointermove}
          {onpointerup}
        ></div>
      {/if}
    {/each}
  {/if}
</dialog>

<style>
  .sheet {
    margin: 12vh auto auto;
    padding: 0;
    color: inherit;
    overflow: visible;
    max-width: calc(100vw - 32px);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
  }
  .sheet::backdrop {
    background: var(--scrim);
  }
  .sheet:focus-visible {
    outline: none;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 36px;
    padding: 0 8px 0 16px;
    flex: none;
    border-bottom: 1px solid var(--line-soft);
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
    font-weight: 600;
  }
  .bar {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin: 0 8px 0 16px;
  }
  .sheet.resizable {
    max-height: calc(100vh - 12vh - 16px);
  }
  /* Content that lays out its own panes fills the dialog and scrolls them itself. */
  .content {
    flex: 1 1 auto;
    min-height: 0;
  }
  /* Grips on the sides and bottom, straddling the border; the top stays put. */
  .edge {
    position: absolute;
    touch-action: none;
  }
  .edge:focus-visible {
    outline: none;
    background: var(--focus);
  }
  .left,
  .right {
    top: 0;
    bottom: 8px;
    width: var(--w-splitter);
    cursor: ew-resize;
  }
  .left {
    left: calc(var(--w-splitter) / -2);
  }
  .right {
    right: calc(var(--w-splitter) / -2);
  }
  .bottom {
    left: 8px;
    right: 8px;
    bottom: calc(var(--w-splitter) / -2);
    height: var(--w-splitter);
    cursor: ns-resize;
  }
  .bottom-left,
  .bottom-right {
    bottom: calc(var(--w-splitter) / -2);
    width: 12px;
    height: 12px;
  }
  .bottom-left {
    left: calc(var(--w-splitter) / -2);
    cursor: nesw-resize;
  }
  .bottom-right {
    right: calc(var(--w-splitter) / -2);
    cursor: nwse-resize;
  }
  /* Less on the right, where the scroll bar's gutter adds its own. */
  .padded {
    padding: 16px calc(16px - var(--w-scrollbar)) 16px 16px;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 16px;
    flex: none;
    border-top: 1px solid var(--line-soft);
  }
</style>

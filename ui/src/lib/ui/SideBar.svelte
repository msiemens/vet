<script lang="ts">
  import type { Snippet } from "svelte";
  import Splitter from "./Splitter.svelte";

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

  let panel: HTMLElement;
  let width = $state<number>();

  /** The token-defined limits, as the browser resolves them. */
  function limits(): [number, number] {
    const style = getComputedStyle(panel);
    return [parseFloat(style.minWidth), parseFloat(style.maxWidth)];
  }
</script>

<aside class={side} bind:this={panel} style:width={width === undefined ? defaultWidth : `${width}px`}>
  {@render children()}
  <!-- Each side keeps its own width. -->
  <Splitter
    orientation="vertical"
    label="Resize sidebar"
    storageKey="vet.sidebar.{side}"
    direction={side === "left" ? 1 : -1}
    current={() => panel.offsetWidth}
    {limits}
    bind:size={width}
    style="{side === 'left' ? 'right' : 'left'}: calc(var(--w-splitter) / -2)"
  />
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
    background: var(--bg-panel);
  }
  .left {
    border-right: 1px solid var(--line);
  }
  .right {
    border-left: 1px solid var(--line);
  }
</style>

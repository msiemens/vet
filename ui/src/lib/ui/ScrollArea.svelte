<!-- A vertically scrolling area with Vet's own scroll bar (`scrollbar.ts`) in a gutter beside it.
     It grows to fill a flex column and shrinks below its content, so it fills a panel or, under a
     `max-height`, ends where its content does. The rest props go to the scrolling element, `viewport`. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import { Scrollbar } from "./scrollbar";

  let {
    viewport = $bindable(),
    children,
    ...rest
  }: {
    /** The scrolling element. */
    viewport?: HTMLElement;
    children: Snippet;
  } & HTMLAttributes<HTMLDivElement> = $props();

  function bar(area: HTMLElement) {
    const scroller = area.firstElementChild as HTMLElement;
    const scrollbar = new Scrollbar(scroller);
    area.append(scrollbar.dom);
    return () => scrollbar.destroy();
  }
</script>

<div class="area" {@attach bar}>
  <div class="viewport" bind:this={viewport} {...rest}>{@render children()}</div>
</div>

<style>
  .area {
    flex: 1 1 auto;
    min-height: 0;
    min-width: 0;
    display: flex;
  }
  .viewport {
    flex: 1;
    min-width: 0;
    overflow: hidden auto;
  }
</style>

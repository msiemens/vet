<script lang="ts">
  // The bottom dock: the one place the bottom tool windows (Timeline, Problems, …) sit under the
  // editors. It owns the height and the splitter above it; a tool's panel fills it.
  import type { Snippet } from "svelte";
  import Splitter from "../../lib/ui/Splitter.svelte";
  import { DOCK_MIN, dockLimits } from "./dock";

  let { children }: { children: Snippet } = $props();

  let height = $state<number>();
  let root = $state<HTMLElement>();
</script>

<div class="dock" bind:this={root} style:height={height === undefined ? undefined : `${height}px`}>
  <Splitter
    orientation="horizontal"
    label="Resize the bottom panel"
    storageKey="vet.dock"
    direction={-1}
    current={() => root?.offsetHeight ?? DOCK_MIN}
    limits={() => dockLimits(root?.parentElement?.clientHeight ?? 0)}
    bind:size={height}
    style="top: calc(var(--w-splitter) / -2)"
  />
  {@render children()}
</div>

<style>
  .dock {
    position: relative;
    flex: none;
    height: var(--h-dock);
    display: flex;
    flex-direction: column;
  }
</style>

<script lang="ts">
  import type { Snippet } from "svelte";

  let { children }: { children?: Snippet } = $props();
</script>

<!-- The window's title bar: empty space drags the window, and the platform's window
     controls (drawn over it by the host) get the clearance they publish. -->
<header data-tauri-drag-region>{@render children?.()}</header>

<style>
  header {
    display: flex;
    flex-wrap: nowrap;
    align-items: center;
    gap: 8px;
    height: var(--h-header);
    padding: 0 max(10px, var(--tauri-plugin-decoration-right-clearance, 0px)) 0
      max(10px, var(--tauri-plugin-decoration-left-clearance, 0px));
    box-sizing: border-box;
    overflow: hidden;
    background: var(--bg-window);
    border-bottom: 1px solid var(--line);
    flex: none;
  }
</style>

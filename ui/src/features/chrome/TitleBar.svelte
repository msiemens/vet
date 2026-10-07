<script lang="ts">
  import type { Snippet } from "svelte";
  import { isMac } from "../../state/host";
  import { mainMenu } from "../../state/mainMenu.svelte";
  import AppLogo from "../../lib/ui/AppLogo.svelte";
  import MainMenuButton from "./MainMenuButton.svelte";

  let { children }: { children?: Snippet } = $props();
</script>

<!-- The window's title bar: empty space drags the window, and the platform's window
     controls (drawn over it by the host) get the clearance they publish. Outside macOS (where
     the traffic lights sit at the left) it starts with the app logo; without a native menu bar
     the main menu follows. -->
<header data-tauri-drag-region>
  {#if !isMac}<AppLogo />{/if}
  {#if !mainMenu.native}<MainMenuButton />{/if}
  {@render children?.()}
</header>

<style>
  header {
    display: flex;
    flex-wrap: nowrap;
    align-items: center;
    gap: 8px;
    height: var(--h-header);
    padding: 0 calc(var(--tauri-plugin-decoration-right-clearance, 0px) + 10px) 0
      calc(var(--tauri-plugin-decoration-left-clearance, 0px) + 10px);
    box-sizing: border-box;
    overflow: hidden;
    background: var(--bg-window);
    border-bottom: 1px solid var(--line);
    flex: none;
  }
</style>

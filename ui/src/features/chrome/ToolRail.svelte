<script module lang="ts">
  import type { IconName } from "../../lib/ui/icons";

  export const TOOLS: Record<"left" | "right", { id: string; icon: IconName; label: string }[]> = {
    left: [
      { id: "queue", icon: "queue", label: "Queue" },
      { id: "files", icon: "files", label: "Files" },
      { id: "timeline", icon: "timeline", label: "Timeline" },
      { id: "comments", icon: "comments", label: "Comments" },
    ],
    right: [{ id: "symbol", icon: "module", label: "Symbol" }],
  };
</script>

<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { toggleTool } from "./toggleTool";

  let {
    side = "left",
    active = $bindable(),
    open = $bindable(true),
  }: {
    side?: "left" | "right";
    /** The selected tool; it stays selected while its sidebar is hidden. */
    active?: string;
    /** Whether the selected tool's sidebar is shown. */
    open?: boolean;
  } = $props();

  /** The active tool's icon shows or hides its sidebar; any other icon selects that tool and shows it. */
  function select(id: string) {
    const next = toggleTool({ tool: active ?? "", open }, id);
    [active, open] = [next.tool, next.open];
  }
</script>

<nav class={side} aria-label={side === "left" ? "Tool windows" : "Right tool windows"}>
  {#each TOOLS[side] as tool (tool.id)}
    <IconButton
      icon={tool.icon}
      label={tool.label}
      active={open && tool.id === active}
      onclick={() => select(tool.id)}
    />
  {/each}
</nav>

<style>
  nav {
    width: var(--w-rail);
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px 0;
    box-sizing: border-box;
    background: var(--bg-window);
  }
  .left {
    border-right: 1px solid var(--line);
  }
  .right {
    border-left: 1px solid var(--line);
  }
</style>

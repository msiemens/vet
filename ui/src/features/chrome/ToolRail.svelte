<script module lang="ts">
  import type { IconName } from "../../lib/ui/icons";
  import { TOOL_WINDOWS, toolCommand } from "../../state/appCommands";
  import { commands } from "../../state/commands";

  const ICONS: Record<string, IconName> = {
    changes: "review",
    all: "tree",
    commits: "gitCommit",
    comments: "comments",
    symbol: "module",
  };

  export const TOOLS = {
    left: TOOL_WINDOWS.filter((t) => t.side === "left"),
    right: TOOL_WINDOWS.filter((t) => t.side === "right"),
  };
</script>

<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { toggleTool } from "./toggleTool";

  let {
    side = "left",
    active = $bindable(),
    open = $bindable(true),
    unavailable = {},
    timeline = false,
    ontimeline,
  }: {
    side?: "left" | "right";
    /** Tools that are shown but cannot be picked now, with the reason for the tooltip. */
    unavailable?: Record<string, string>;
    /** The selected tool; it stays selected while its sidebar is hidden. */
    active?: string;
    /** Whether the selected tool's sidebar is shown. */
    open?: boolean;
    /** The timeline panel is shown. */
    timeline?: boolean;
    /** The left rail's icon at the bottom shows or hides the timeline. */
    ontimeline?: () => void;
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
      icon={ICONS[tool.id]}
      label={tool.label}
      title={unavailable[tool.id] ?? commands.tooltip(toolCommand(tool.id))}
      disabled={tool.id in unavailable}
      active={open && tool.id === active}
      onclick={() => select(tool.id)}
    />
  {/each}
  {#if ontimeline}
    <span class="bottom"><IconButton icon="timeline" label="Timeline" active={timeline} onclick={ontimeline} /></span>
  {/if}
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
  .bottom {
    margin-top: auto;
  }
  .left {
    border-right: 1px solid var(--line);
  }
  .right {
    border-left: 1px solid var(--line);
  }
</style>

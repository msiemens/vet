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
    structure: "structure",
  };

  export const TOOLS = {
    left: TOOL_WINDOWS.filter((t) => t.side === "left"),
    right: TOOL_WINDOWS.filter((t) => t.side === "right"),
  };
</script>

<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";

  let {
    side = "left",
    active,
    open = true,
    counts = {},
    timeline,
  }: {
    side?: "left" | "right";
    /** A number on a tool's icon: what is new there. */
    counts?: Record<string, number>;
    /** The selected tool; it stays selected while its sidebar is hidden. */
    active?: string;
    /** Whether the selected tool's sidebar is shown. */
    open?: boolean;
    /** Whether the timeline panel is shown; the left rail has its icon at the bottom, the right one none. */
    timeline?: boolean;
  } = $props();
</script>

<nav class={side} aria-label={side === "left" ? "Tool windows" : "Right tool windows"}>
  {#each TOOLS[side] as tool (tool.id)}
    {@const count = counts[tool.id] ?? 0}
    {@const command = toolCommand(tool.id)}
    <span class="slot">
      <IconButton
        icon={ICONS[tool.id]}
        label={count > 0 ? `${tool.label}, ${count} new` : tool.label}
        title={commands.hint(command)}
        disabled={!commands.isEnabled(command)}
        active={open && tool.id === active}
        onclick={() => commands.run(command)}
      />
      {#if count > 0}<span class="count mono" aria-hidden="true">{count > 99 ? "99+" : count}</span>{/if}
    </span>
  {/each}
  {#if timeline !== undefined}
    <span class="bottom"
      ><IconButton
        icon="timeline"
        label="Timeline"
        title={commands.hint(toolCommand("timeline"))}
        disabled={!commands.isEnabled(toolCommand("timeline"))}
        active={timeline}
        onclick={() => commands.run(toolCommand("timeline"))}
      /></span
    >
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
  /* Not `.tool`: that global class pads the span wider than the rail. */
  .slot {
    position: relative;
    display: inline-flex;
  }
  .count {
    position: absolute;
    top: -3px;
    right: -4px;
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    box-sizing: border-box;
    border-radius: 7px;
    background: var(--primary);
    color: var(--on-primary);
    font-size: 9.5px;
    line-height: 14px;
    text-align: center;
    pointer-events: none;
  }
  .left {
    border-right: 1px solid var(--line);
  }
  .right {
    border-left: 1px solid var(--line);
  }
</style>

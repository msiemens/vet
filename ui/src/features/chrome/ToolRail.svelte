<script module lang="ts">
  import type { IconName } from "../../lib/ui/icons";
  import { TOOL_WINDOWS, toolCommand } from "../../state/appCommands";
  import { commands } from "../../state/commands";

  const ICONS: Record<string, IconName> = {
    changes: "review",
    all: "tree",
    commits: "gitCommit",
    comments: "comments",
    timeline: "timeline",
    problems: "problems",
    pipelines: "pipelines",
    symbol: "module",
    structure: "structure",
  };

  export const TOOLS = {
    left: TOOL_WINDOWS.filter((t) => t.side === "left"),
    bottom: TOOL_WINDOWS.filter((t) => t.side === "bottom"),
    right: TOOL_WINDOWS.filter((t) => t.side === "right"),
  };
</script>

<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";

  let {
    side = "left",
    active,
    open = true,
    badges = {},
    bottom,
  }: {
    side?: "left" | "right";
    /** A number on a tool's icon, with what it counts in words for a screen reader ("3 new"). */
    badges?: Partial<Record<string, { count: number; text: string; bad?: boolean }>>;
    /** The selected tool; it stays selected while its sidebar is hidden. */
    active?: string;
    /** Whether the selected tool's sidebar is shown. */
    open?: boolean;
    /** The bottom tool whose panel is shown, if one is; the left rail has the bottom tools' icons, the right one none. */
    bottom?: string | null;
  } = $props();
</script>

{#snippet slot(tool: { id: string; label: string }, shown: boolean)}
  {@const badge = badges[tool.id]}
  {@const count = badge?.count ?? 0}
  {@const command = toolCommand(tool.id)}
  <span class="slot">
    <IconButton
      icon={ICONS[tool.id]}
      label={badge && count > 0 ? `${tool.label}, ${badge.text}` : tool.label}
      title={commands.hint(command)}
      disabled={!commands.isEnabled(command)}
      active={shown}
      onclick={() => commands.run(command)}
    />
    {#if count > 0}<span class="count mono" class:bad={badge?.bad} aria-hidden="true">{count > 99 ? "99+" : count}</span
      >{/if}
  </span>
{/snippet}

<nav class={side} aria-label={side === "left" ? "Tool windows" : "Right tool windows"}>
  {#each TOOLS[side] as tool (tool.id)}
    {@render slot(tool, open && tool.id === active)}
  {/each}
  {#if bottom !== undefined}
    <span class="bottom">
      {#each TOOLS.bottom as tool (tool.id)}
        {@render slot(tool, tool.id === bottom)}
      {/each}
    </span>
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
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
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
  .count.bad {
    background: var(--del-text);
  }
  .left {
    border-right: 1px solid var(--line);
  }
  .right {
    border-left: 1px solid var(--line);
  }
</style>

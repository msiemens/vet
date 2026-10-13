<script lang="ts">
  import { tick } from "svelte";
  import type { RecentProject } from "../../generated/RecentProject";
  import Chip from "../../lib/ui/Chip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Marked from "../../lib/ui/Marked.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { recentMenu } from "./recentCommands";
  import ProjectAvatar from "./ProjectAvatar.svelte";
  import { searchRecent } from "./recentList";

  let {
    name,
    path,
    recent,
    host,
    canPick,
  }: {
    /** The open project's name and path. */
    name: string;
    path: string;
    /** Backend-side paths with when they were opened, most recent first. */
    recent: RecentProject[];
    /** Where the projects live (the backend's host), if it is worth saying. */
    host?: string;
    /** A native folder dialog exists. */
    canPick: boolean;
  } = $props();

  let open = $state(false);
  let query = $state("");
  let cursor = $state(0);
  let list = $state<HTMLElement>();

  const found = $derived(searchRecent(recent, query));
  // The Open… row, when there is one, comes after the projects and is reached the same way.
  const stops = $derived(found.length + (canPick ? 1 : 0));
  const current = $derived(Math.min(cursor, stops - 1));
  const onOpenRow = $derived(canPick && current === found.length);

  // Each opening starts with the whole list.
  $effect(() => {
    if (open) [query, cursor] = ["", 0];
  });

  const focus = (node: HTMLElement) => node.focus();

  async function move(by: number) {
    cursor = Math.max(0, Math.min(current + by, stops - 1));
    await tick();
    list?.querySelector(".cursor")?.scrollIntoView({ block: "nearest" });
  }

  function choose(project: string) {
    open = false;
    if (project !== path) commands.run(`file.openRecent:${project}`);
  }

  /** A right click puts the cursor on the row, and opens its menu. */
  function openMenu(e: MouseEvent, project: string, index: number) {
    e.preventDefault();
    cursor = index;
    void contextMenus.open(recentMenu, e.clientX, e.clientY, { recentProject: { path: project } });
  }

  function pick() {
    open = false;
    commands.run("file.open");
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      void move(e.key === "ArrowDown" ? 1 : -1);
    } else if (e.key === "Enter" && stops) {
      e.preventDefault();
      if (onOpenRow) pick();
      else choose(found[current].project.path);
    }
  }
</script>

<Popover bind:open>
  {#snippet trigger(toggle)}
    <Chip bold active={open} onclick={toggle}>{name}</Chip>
  {/snippet}
  <div class="switcher">
    <label class="search">
      <Icon name="search" size={13} />
      <input
        class="field"
        type="text"
        use:focus
        bind:value={query}
        oninput={() => (cursor = 0)}
        {onkeydown}
        aria-label="Search projects"
        spellcheck="false"
      />
    </label>
    <ul bind:this={list} aria-label="Recent projects">
      {#each found as { project, name: projectName, nameRanges, pathRanges }, i (project.path)}
        <li>
          <button
            class="flat row"
            class:on={project.path === path}
            class:cursor={i === current}
            type="button"
            tabindex="-1"
            title={project.path}
            onclick={() => choose(project.path)}
            oncontextmenu={(e) => openMenu(e, project.path, i)}
          >
            <ProjectAvatar path={project.path} />
            <span class="text">
              <span class="name"><Marked text={projectName} ranges={nameRanges} /></span>
              <span class="mono path"><PathText path={project.path} ranges={pathRanges} /></span>
            </span>
            {#if host}<span class="host">{host}</span>{/if}
            <span class="check"
              >{#if project.path === path}<Icon name="check" size={13} stroke={2.2} />{/if}</span
            >
          </button>
        </li>
      {:else}
        <li class="empty">{recent.length ? "No matching projects" : "No recent projects"}</li>
      {/each}
    </ul>
    {#if canPick}
      <div class="foot">
        <button
          class="flat"
          class:cursor={onOpenRow}
          type="button"
          tabindex="-1"
          title={commands.hint("file.open")}
          disabled={!commands.isEnabled("file.open")}
          onclick={pick}>Open… <span class="key">{commands.binding("file.open")?.label}</span></button
        >
      </div>
    {/if}
  </div>
</Popover>

<style>
  .switcher {
    width: var(--w-switcher);
    display: flex;
    flex-direction: column;
  }
  .search {
    position: relative;
    display: flex;
    align-items: center;
    margin: 8px;
    color: var(--text-3);
  }
  .search :global(svg) {
    position: absolute;
    left: 8px;
    pointer-events: none;
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding-left: 26px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 4px 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: min(60vh, 420px);
    overflow: hidden auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 44px;
    padding: 0 8px;
    box-sizing: border-box;
    border-radius: var(--radius-sheet);
    text-align: left;
  }
  .row.on {
    background: var(--toggle);
  }
  .row.cursor:not(.on) {
    background: var(--hover);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-ctl);
  }
  .path {
    display: flex;
    font-size: var(--fs-mono-s);
  }
  .host {
    flex: none;
    font-size: var(--fs-mono-s);
    color: var(--text-2);
  }
  .check {
    flex: none;
    width: 14px;
    display: inline-flex;
    color: var(--accent);
  }
  .empty {
    padding: 8px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
  .foot {
    padding: 4px;
    border-top: 1px solid var(--line-soft);
  }
  .foot button {
    width: 100%;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--h-row);
    padding: 0 8px;
    border-radius: var(--radius-sheet);
    font-size: var(--fs-ctl);
  }
  .foot button.cursor {
    background: var(--hover);
  }
  .key {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
</style>

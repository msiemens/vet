<script lang="ts">
  import { type Snippet, tick } from "svelte";
  import type { RecentProject } from "../../generated/RecentProject";
  import AppLogo from "../../lib/ui/AppLogo.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Marked from "../../lib/ui/Marked.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { shortTime } from "../../lib/shortTime";
  import { commands } from "../../state/commands";
  import { avatarColour, initials, searchRecent } from "./recentList";

  // It draws its own window-controls area, and its buttons run the window's commands.
  let {
    menu,
    recent,
    failed,
    canPick,
    version,
    onforget,
  }: {
    /** The application menu, where the window draws it itself; at the top of the sidebar. */
    menu?: Snippet;
    /** Backend-side paths with when they were opened, most recent first. */
    recent: RecentProject[];
    /** The last open attempt that failed; `path` is empty when the folder dialog itself refused. */
    failed?: { path: string; message: string };
    /** A native folder dialog exists; otherwise a typed path is opened with Enter. */
    canPick: boolean;
    version?: string;
    onforget: (path: string) => void;
  } = $props();

  const open = (path: string) => commands.run(`file.openRecent:${path}`);

  let query = $state("");
  let selected = $state(0);
  let list: HTMLElement | undefined;

  const found = $derived(searchRecent(recent, query));
  const typed = $derived(query.trim());
  // The highlighted row stays within the list as it narrows.
  const current = $derived(Math.min(selected, found.length - 1));

  async function move(by: number) {
    selected = Math.max(0, Math.min(current + by, found.length - 1));
    await tick();
    list?.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      void move(e.key === "ArrowDown" ? 1 : -1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (found.length) open(found[current].project.path);
      else if (!canPick && typed) open(typed);
    } else if (e.key === "Escape") {
      query = "";
    }
  }

  function focus(node: HTMLElement) {
    node.focus();
  }
</script>

<div class="welcome">
  <nav class="side" aria-label="Welcome">
    <div class="controls" data-tauri-drag-region>{@render menu?.()}</div>
    <div class="brand">
      <span class="tile"><AppLogo size={26} /></span>
      <span class="title">
        <span class="app">Vet</span>
        {#if version}<span class="mono version">{version}</span>{/if}
      </span>
    </div>
    <ul>
      <li>
        <button class="flat item on" type="button" aria-current="page"><Icon name="files" size={16} />Projects</button>
      </li>
      <li>
        <button
          class="flat item"
          type="button"
          title={commands.hint("app.accounts")}
          disabled={!commands.isEnabled("app.accounts")}
          onclick={() => commands.run("app.accounts")}><Icon name="user" size={16} />Accounts</button
        >
      </li>
      <li>
        <button
          class="flat item"
          type="button"
          title={commands.hint("app.settings")}
          disabled={!commands.isEnabled("app.settings")}
          onclick={() => commands.run("app.settings")}><Icon name="settings" size={16} />Settings</button
        >
      </li>
    </ul>
  </nav>

  <main>
    <div class="bar" data-tauri-drag-region>
      <label class="search">
        <Icon name="search" size={15} />
        <input
          class="field"
          type="text"
          use:focus
          bind:value={query}
          oninput={() => (selected = 0)}
          {onkeydown}
          placeholder={canPick ? "Search projects" : "Search projects or enter a path"}
          aria-label="Search projects"
          spellcheck="false"
        />
      </label>
      {#if canPick}
        <button
          class="ctl"
          type="button"
          title={commands.hint("file.open")}
          disabled={!commands.isEnabled("file.open")}
          onclick={() => commands.run("file.open")}>Open…</button
        >
      {/if}
      <button class="ctl" type="button" disabled title="Coming later">Clone…</button>
    </div>

    {#if failed}
      <div class="error" role="alert">
        <span class="selectable">{failed.message}</span>
        {#if recent.some((p) => p.path === failed.path)}
          <button class="ctl" type="button" onclick={() => onforget(failed.path)}>Remove from recent</button>
        {/if}
      </div>
    {/if}

    <ScrollArea>
      <ul class="projects" bind:this={list} role="listbox" aria-label="Recent projects">
        {#each found as { project, name, nameRanges, pathRanges }, i (project.path)}
          <li role="option" aria-selected={i === current}>
            <button
              class="flat row"
              class:on={i === current}
              type="button"
              tabindex="-1"
              title={project.path}
              onclick={() => open(project.path)}
            >
              <span class="avatar" style:background="var(--avatar-{avatarColour(project.path)})">{initials(name)}</span>
              <span class="text">
                <span class="name"><Marked text={name} ranges={nameRanges} /></span>
                <span class="mono path"><PathText path={project.path} ranges={pathRanges} /></span>
              </span>
              <span class="when">{shortTime(project.openedAt)}</span>
            </button>
          </li>
        {/each}
      </ul>
      {#if !found.length}
        <p class="empty">{recent.length ? "No matching projects" : "No recent projects"}</p>
      {/if}
    </ScrollArea>
  </main>
</div>

<style>
  .welcome {
    flex: 1;
    min-height: 0;
    display: flex;
    background: var(--bg-editor);
  }
  .side {
    flex: none;
    width: var(--w-welcome-side);
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-right: 1px solid var(--line);
  }
  /* Where the platform's window controls sit (macOS traffic lights); the sidebar drags the window. */
  .controls {
    flex: none;
    height: var(--h-header);
    display: flex;
    align-items: center;
    padding: 0 10px 0 calc(var(--tauri-plugin-decoration-left-clearance, 0px) + 10px);
    box-sizing: border-box;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px 16px;
  }
  .tile {
    flex: none;
    width: var(--s-logo);
    height: var(--s-logo);
    display: grid;
    place-items: center;
    border-radius: var(--radius-sheet);
    background: var(--logo-tile);
  }
  .title {
    display: flex;
    flex-direction: column;
    line-height: 1.25;
  }
  .app {
    font-size: 15px;
    font-weight: 600;
  }
  .version {
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .side ul {
    padding: 0 12px;
  }
  .item {
    width: 100%;
    height: var(--h-ctl);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    border-radius: var(--radius);
    text-align: left;
    font-size: var(--fs);
  }
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px calc(var(--tauri-plugin-decoration-right-clearance, 0px) + 12px) 8px 12px;
  }
  .search {
    flex: 1;
    min-width: 0;
    position: relative;
    display: flex;
    align-items: center;
    color: var(--text-3);
  }
  .search :global(svg) {
    position: absolute;
    left: 9px;
    pointer-events: none;
  }
  .search input {
    flex: 1;
    padding-left: 30px;
  }
  .error {
    flex: none;
    box-sizing: border-box;
    margin: 0 12px 8px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--del-text);
    border-radius: var(--radius);
    background: var(--del-bg);
    font-size: var(--fs-ctl);
  }
  /* The scroll bar's gutter is the margin on the right. */
  .projects {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 0 0 12px;
  }
  .row {
    width: 100%;
    height: var(--h-welcome-row);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    border-radius: var(--radius-sheet);
    text-align: left;
  }
  .avatar {
    flex: none;
    width: var(--s-avatar);
    height: var(--s-avatar);
    display: grid;
    place-items: center;
    border-radius: var(--radius-sheet);
    color: var(--on-avatar);
    font-size: 12px;
    font-weight: 600;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    font-size: var(--fs);
  }
  .path {
    display: flex;
    font-size: var(--fs-mono-s);
  }
  /* The selection is a highlight, not a toggled control: its text keeps the text colour. */
  .row.on,
  .item.on {
    color: var(--text);
  }
  .when {
    flex: none;
    font-size: var(--fs-ctl);
    color: var(--text-2);
  }
  .empty {
    margin: 0;
    padding: 12px 24px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
</style>

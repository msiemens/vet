<script lang="ts">
  import type { Snippet } from "svelte";
  import type { RecentProject } from "../../generated/RecentProject";
  import AppLogo from "../../lib/ui/AppLogo.svelte";
  import Banner from "../../lib/ui/Banner.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { ListNav, revealCurrent } from "../../lib/ui/listNav";
  import PickList from "../../lib/ui/PickList.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import SearchField from "../../lib/ui/SearchField.svelte";
  import { shortTime } from "../../lib/shortTime";
  import { commands } from "../../state/commands";
  import { minute } from "../../state/minute.svelte";
  import { openRecent, openRecentMenu } from "./openRecent";
  import RecentRow from "./RecentRow.svelte";
  import { type RecentMatch, searchRecent } from "./recentList";

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

  let query = $state("");
  let cursor = $state(0);
  let list = $state<HTMLElement>();

  const found = $derived(searchRecent(recent, query));
  const typed = $derived(query.trim());
  // The highlighted row stays within the list as it narrows.
  const current = $derived(Math.min(cursor, found.length - 1));

  const nav = new ListNav<RecentMatch>({
    items: () => found,
    single: (match) => openRecent(match.project.path),
    highlight: { get: () => current, set: (i) => (cursor = i) },
    reveal: () => void revealCurrent(list),
    field: true,
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !found.length && !canPick && typed) {
      e.preventDefault();
      openRecent(typed);
    } else if (nav.keydown(e)) e.preventDefault();
    else if (e.key === "Escape") query = "";
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
        <button class="flat pick item on" type="button" aria-current="page"
          ><Icon name="files" size={16} />Projects</button
        >
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
      <SearchField
        autofocus
        bind:value={query}
        oninput={() => (cursor = 0)}
        {onkeydown}
        placeholder={canPick ? "Search projects" : "Search projects or enter a path"}
        label="Search projects"
      />
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
      <Banner
        tone="bad"
        message={failed.message}
        actions={recent.some((p) => p.path === failed.path)
          ? [{ label: "Remove from recent", onclick: () => onforget(failed.path) }]
          : undefined}
      />
    {/if}

    <ScrollArea>
      <div class="projects">
        <PickList
          bind:list
          items={found}
          key={(match: RecentMatch) => match.project.path}
          cursor={current}
          label="Recent projects"
          empty={recent.length ? "No matching projects" : "No recent projects"}
          --pick-empty-pad="12px 24px"
          onpick={(_, index, e) => nav.click(e, index)}
          onmenu={(match, index, e) => {
            cursor = index;
            openRecentMenu(e, match.project.path);
          }}
        >
          {#snippet row(match)}
            <RecentRow {match} roomy>
              {#snippet trailing()}<span class="when">{shortTime(match.project.openedAt, minute.now)}</span>{/snippet}
            </RecentRow>
          {/snippet}
        </PickList>
      </div>
    </ScrollArea>
  </main>
</div>

<style>
  .welcome {
    flex: 1;
    min-width: 0;
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
    font-size: var(--fs-title);
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
    display: flex;
    flex-direction: column;
    gap: 2px;
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
  /* The scroll bar's gutter is the margin on the right. */
  .projects {
    padding: 0 0 0 12px;
  }
  .when {
    flex: none;
    font-size: var(--fs-ctl);
    color: var(--text-2);
  }
</style>

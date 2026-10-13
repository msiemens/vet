<script lang="ts">
  import type { CommitUnit } from "../../generated/CommitUnit";
  import type { FileLayout } from "../../state/reviewModel";
  import { basename } from "../../lib/paths";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Tab from "../../lib/ui/Tab.svelte";
  import { type TabId, commands } from "../../state/commands";
  import { changesTitle, commitOf, needsCommitName, pathOf, scrollToReveal } from "./tabModel";
  import type { Tabs } from "./Tabs.svelte";

  /**
   * `files` are the ones the first tab lists, which it names by `scope`, what they are of; its tooltip
   * counts them and, by commit, names `commit`; `unitOf` is the commit a file tab stays at, once it is read; `onmenu` opens the context menu of a tab.
   */
  let {
    tabs,
    files,
    scope,
    commit,
    unitOf,
    onmenu,
  }: {
    tabs: Tabs;
    files: readonly FileLayout[];
    scope: string;
    commit?: CommitUnit;
    unitOf: (id: TabId) => CommitUnit | undefined;
    onmenu: (id: TabId, x: number, y: number) => void;
  } = $props();

  /** Closing a tab is the Close Tab command, with the tab as its subject. */
  const closing = (id: TabId) => commands.context({ tab: id });

  /** What a file tab's tooltip says: the path, and for a tab at a commit the commit in full. */
  function titleOf(id: TabId, path: string): string {
    const at = commitOf(id);
    if (at === undefined) return path;
    const subject = unitOf(id)?.commit.subject;
    return `${path}\nAs of ${at}${subject === undefined ? "" : ` ${subject}`}`;
  }

  let strip: HTMLElement | undefined = $state();

  /** Scrolls the strip, never the page, so the active tab shows whole. */
  function revealActive() {
    const el = strip?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.parentElement;
    if (!strip || !el) return;
    const at = el.getBoundingClientRect().left - strip.getBoundingClientRect().left + strip.scrollLeft;
    strip.scrollLeft = scrollToReveal(strip.scrollLeft, strip.clientWidth, at, el.offsetWidth);
  }

  // Opening or closing a tab shifts the active one, so the list is watched with the active tab.
  $effect(() => {
    if (tabs.open.includes(tabs.active)) revealActive();
  });

  $effect(() => {
    if (!strip) return;
    const observer = new ResizeObserver(revealActive);
    observer.observe(strip);
    return () => observer.disconnect();
  });
</script>

<div class="tabs" role="tablist" aria-label="Open views" bind:this={strip}>
  {#each tabs.open as id (id)}
    {@const path = pathOf(id)}
    {@const log = tabs.logs.get(id)}
    {@const subject = closing(id)}
    <Tab
      selected={tabs.active === id}
      title={log
        ? (log.hint ?? log.title)
        : path !== undefined
          ? titleOf(id, path)
          : changesTitle(files.length, commit?.commit.subject)}
      onselect={() => (tabs.active = id)}
      onclose={commands.isEnabled("tab.close", subject) ? () => commands.run("tab.close", subject) : undefined}
      closeLabel={tabs.active === id ? commands.tooltip("tab.close") : commands.label("tab.close")}
      onmenu={(x, y) => onmenu(id, x, y)}
    >
      {#if log}
        <Icon name="file" size={12} />
        {log.title}
      {:else if path === undefined}
        {scope}
      {:else}
        {@const at = commitOf(id)}
        <FileIcon name={basename(path)} />
        {basename(path)}
        {#if at !== undefined}
          {@const unit = unitOf(id)}
          <span class="at">
            <Icon name={unit?.kind.type === "merge" ? "merge" : "commit"} size={12} />
            {#if needsCommitName(tabs.open, id)}
              <span class="mono">{unit?.commit.shortId ?? at.slice(0, 7)}</span>
            {/if}
          </span>
        {/if}
      {/if}
    </Tab>
  {/each}
</div>

<style>
  .tabs {
    display: flex;
    align-items: flex-end;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .at {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent);
  }
  .at .mono {
    color: var(--text-2);
    font-size: var(--fs-mono-s);
  }
</style>

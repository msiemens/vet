<script lang="ts">
  import type { CommitUnit } from "../../generated/CommitUnit";
  import type { FileChange } from "../../generated/FileChange";
  import { basename } from "../../lib/paths";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Tab from "../../lib/ui/Tab.svelte";
  import { commands } from "../../state/commands";
  import { type TabId, type Tabs, kindOf, pathOf, subjectOf } from "./Tabs.svelte";

  /**
   * `files` are the ones the first tab lists, or the commit's, which the first tab then names by its hash and
   * subject; `onmenu` opens the context menu of a tab.
   */
  let {
    tabs,
    files,
    commit,
    onmenu,
  }: { tabs: Tabs; files: FileChange[]; commit?: CommitUnit; onmenu: (id: TabId, x: number, y: number) => void } =
    $props();

  /** Closing a tab is the Close Tab command, with the tab as its subject. */
  const closing = (id: TabId) => commands.context({ tab: { kind: kindOf(id), path: subjectOf(id) } });
</script>

<div class="tabs" role="tablist" aria-label="Open views">
  {#each tabs.open as id (id)}
    {@const path = pathOf(id)}
    {@const log = tabs.logs.get(id)}
    {@const subject = closing(id)}
    <Tab
      selected={tabs.active === id}
      title={log ? (log.hint ?? log.title) : (path ?? (commit ? commit.commit.subject : "Every changed file"))}
      onselect={() => (tabs.active = id)}
      onclose={commands.isEnabled("tab.close", subject) ? () => commands.run("tab.close", subject) : undefined}
      closeLabel={tabs.active === id ? commands.tooltip("tab.close") : commands.label("tab.close")}
      onmenu={(x, y) => onmenu(id, x, y)}
    >
      {#if log}
        <Icon name="file" size={12} />
        {log.title}
      {:else if path === undefined && commit}
        <Icon name={commit.kind.type === "merge" ? "merge" : "commit"} size={12} />
        <span class="mono hash">{commit.commit.shortId}</span>
        <span class="subject">{commit.commit.subject}</span>
      {:else if path === undefined}
        All changes <span class="count">{files.length}</span>
      {:else}
        <FileIcon name={basename(path)} />
        {basename(path)}
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
  .count {
    margin-left: 4px;
    color: var(--text-3);
  }
  .hash {
    margin: 0 6px 0 4px;
    color: var(--text-2);
  }
  .subject {
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>

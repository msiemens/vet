<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import Icon from "../../lib/ui/Icon.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import type { FileChange } from "../../generated/FileChange";
  import { type DirRow, buildRows, buildTree, summarize } from "./tree";

  let {
    paths,
    fileOf,
    selected,
    excluded,
    onselect,
    onopen,
    onmenu,
  }: {
    /** Changes only with the changeset's revision, so the tree is built once per revision. */
    paths: string[];
    /** A file's current state; reading it subscribes to that file alone. */
    fileOf: (path: string) => FileChange | undefined;
    selected?: string;
    /** Changes hidden by the review filter, collapsed into one group. */
    excluded?: { lines: number; items: number };
    /** Single click. */
    onselect: (path: string) => void;
    /** Double click or Enter. */
    onopen: (path: string) => void;
    /** Context menu on a file or folder row, at the pointer position. */
    onmenu: (path: string, x: number, y: number) => void;
  } = $props();

  const collapsed = new SvelteSet<string>();
  const tree = $derived(buildTree(paths));
  const rows = $derived(buildRows(tree, collapsed));

  const STATUS = { M: "var(--old-base)", A: "var(--add-text)", D: "var(--del-text)", R: "var(--text-2)" } as const;

  /** Collapsing a folder collapses everything below it too; expanding opens only the folder. */
  function toggle(row: DirRow) {
    if (collapsed.delete(row.path)) return;
    collapsed.add(row.path);
    for (const path of row.descendants) collapsed.add(path);
  }

  function menu(e: MouseEvent, path: string) {
    e.preventDefault();
    onmenu(path, e.clientX, e.clientY);
  }
</script>

<nav aria-label="Changed files">
  <div class="sh"><span>Files</span><span class="grow"></span><span class="mono count">{paths.length}</span></div>
  <div class="rows">
    {#each rows as row (row.path + row.kind)}
      {#if row.kind === "dir"}
        <button
          class="flat tw dir"
          type="button"
          title={row.path}
          style:padding-left="calc(8px + {row.depth} * var(--w-indent))"
          aria-expanded={!collapsed.has(row.path)}
          onclick={() => toggle(row)}
          oncontextmenu={(e) => menu(e, row.path)}
        >
          <span class="chev"><Icon name={collapsed.has(row.path) ? "closed" : "open"} size={10} /></span>
          <span class="name">{row.name}</span>
        </button>
      {:else}
        {@const file = fileOf(row.path)}
        {@const s = file && summarize(file)}
        <button
          class="flat tw"
          class:on={row.path === selected}
          type="button"
          title={row.path}
          style:padding-left="calc(8px + {row.depth} * var(--w-indent))"
          onclick={() => onselect(row.path)}
          ondblclick={() => onopen(row.path)}
          onkeydown={(e) => {
            if (e.key !== "Enter") return;
            e.preventDefault();
            onopen(row.path);
          }}
          oncontextmenu={(e) => menu(e, row.path)}
        >
          <span class="chev"></span>
          {#if s?.build}<span class="build" title="Build file"><Icon name="build" size={12} stroke={2} /></span>{/if}
          <span class="name">{row.name}</span>
          {#if s?.mode}<span class="mono x" title="File mode changed">{s.mode}</span>{/if}
          <Meter
            width={22}
            segments={[
              { value: s?.seenPct ?? 0, color: "var(--seen)" },
              { value: s?.oldPct ?? 0, color: "var(--old-base)" },
            ]}
          />
          <span class="mono status" style:color={s && STATUS[s.status]}>{s?.status}</span>
        </button>
      {/if}
    {/each}
    {#if excluded && excluded.lines + excluded.items > 0}
      <div
        class="tw excluded"
        title="{excluded.lines} changed lines and {excluded.items} other changes excluded by the review filter"
      >
        <span class="chev"><Icon name="closed" size={10} /></span><Icon
          name="filter"
          size={11}
          stroke={2}
        />Excluded<span class="mono x">{excluded.lines}</span>
      </div>
    {/if}
  </div>
</nav>

<style>
  nav {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .sh {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-ctl);
    padding: 0 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
    border-bottom: 1px solid var(--line-soft);
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .count {
    font-weight: 400;
    color: var(--text-3);
  }
  .rows {
    padding: 4px 0;
    overflow: hidden auto;
    min-height: 0;
  }
  .tw {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-row);
    width: 100%;
    padding-right: 10px;
    text-align: left;
    white-space: nowrap;
    font-size: var(--fs-ctl);
    box-sizing: border-box;
  }
  .dir {
    color: var(--text-2);
  }
  /* A fixed column, so names line up whether or not a row has a chevron. */
  .chev {
    flex: none;
    width: var(--w-chev);
    display: inline-flex;
    justify-content: center;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .x {
    flex: none;
    font-size: 11px;
    color: var(--text-2);
  }
  .status {
    flex: none;
    width: 10px;
    font-size: 11px;
  }
  .build {
    flex: none;
    display: inline-flex;
    color: var(--warn);
  }
  .excluded {
    padding-left: 8px;
    color: var(--text-3);
  }
</style>

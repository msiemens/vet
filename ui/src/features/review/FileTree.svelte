<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import Icon from "../../lib/ui/Icon.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import type { FileChange } from "../../generated/FileChange";
  import { type DirRow, type FileRow, buildRows, buildTree } from "./tree";

  let {
    files,
    selected,
    excluded,
    onselect,
    onopen,
    onmenu,
  }: {
    files: FileChange[];
    selected?: string;
    /** Changes hidden by the review filter, collapsed into one group. */
    excluded?: { lines: number; items: number };
    /** Single click. */
    onselect: (row: FileRow) => void;
    /** Double click or Enter. */
    onopen: (row: FileRow) => void;
    /** Context menu on a file or folder row, at the pointer position. */
    onmenu: (path: string, x: number, y: number) => void;
  } = $props();

  const collapsed = new SvelteSet<string>();
  const tree = $derived(buildTree(files));
  const rows = $derived(buildRows(tree, collapsed));

  const STATUS = { M: "var(--old)", A: "var(--addt)", D: "var(--delt)", R: "var(--tx2)" } as const;

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
  <div class="sh"><span>Files</span><span class="grow"></span><span class="mono count">{files.length}</span></div>
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
        <button
          class="flat tw"
          class:on={row.path === selected}
          type="button"
          title={row.path}
          style:padding-left="calc(8px + {row.depth} * var(--w-indent))"
          onclick={() => onselect(row)}
          ondblclick={() => onopen(row)}
          onkeydown={(e) => {
            if (e.key !== "Enter") return;
            e.preventDefault();
            onopen(row);
          }}
          oncontextmenu={(e) => menu(e, row.path)}
        >
          <span class="chev"></span>
          {#if row.build}<span class="build" title="Build file"><Icon name="build" size={12} stroke={2} /></span>{/if}
          <span class="name">{row.name}</span>
          {#if row.mode}<span class="mono x" title="File mode changed">{row.mode}</span>{/if}
          <Meter
            width={22}
            segments={[
              { value: row.seenPct, color: "var(--seen)" },
              { value: row.oldPct, color: "var(--old)" },
            ]}
          />
          <span class="mono status" style:color={STATUS[row.status]}>{row.status}</span>
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
    color: var(--tx2);
    border-bottom: 1px solid var(--ln2);
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .count {
    font-weight: 400;
    color: var(--tx3);
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
    color: var(--tx2);
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
    color: var(--tx2);
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
    color: var(--tx3);
  }
</style>

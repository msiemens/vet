<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { DirEntry } from "../../generated/DirEntry";
  import type { FileChange } from "../../generated/FileChange";
  import Icon from "../../lib/ui/Icon.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { AllFiles } from "./AllFiles.svelte";
  import { buildRows, buildTree, summarize } from "./tree";

  let {
    paths,
    fileOf,
    revision,
    list,
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
    /** The manifest revision; "All files" reloads when it moves. */
    revision?: number;
    /** The children of a directory of the target snapshot, for "All files". */
    list: (path: string) => Promise<{ revision: number; entries: DirEntry[] } | undefined>;
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

  let scope = $state<"changes" | "all">("changes");
  const collapsed = new SvelteSet<string>();
  const tree = $derived(buildTree(paths));
  const rows = $derived(buildRows(tree, collapsed));
  // The list function does not change for a mounted tree.
  // svelte-ignore state_referenced_locally
  const all = new AllFiles(list);

  $effect(() => {
    if (scope === "all" && revision !== undefined) untrack(() => all.reset(revision));
  });

  const STATUS = { M: "var(--old-base)", A: "var(--add-text)", D: "var(--del-text)", R: "var(--text-2)" } as const;

  /** Collapsing a folder collapses everything below it too; expanding opens only the folder. */
  function toggle(path: string, descendants: string[]) {
    if (collapsed.delete(path)) return;
    collapsed.add(path);
    for (const below of descendants) collapsed.add(below);
  }

  function menu(e: MouseEvent, path: string) {
    e.preventDefault();
    onmenu(path, e.clientX, e.clientY);
  }
</script>

{#snippet dirRow(path: string, name: string, depth: number, open: boolean, ontoggle: () => void, entry?: DirEntry)}
  <button
    class="flat tw dir"
    class:dim={entry?.excluded}
    type="button"
    title={path}
    style:padding-left="calc(8px + {depth} * var(--w-indent))"
    aria-expanded={open}
    onclick={ontoggle}
    oncontextmenu={(e) => menu(e, path)}
  >
    <span class="chev"><Icon name={open ? "open" : "closed"} size={10} /></span>
    <span class="name">{name}</span>
    {#if entry?.changed}<span class="mark" title="Contains changes"></span>{/if}
  </button>
{/snippet}

{#snippet fileRow(path: string, name: string, depth: number, entry?: DirEntry)}
  {@const file = fileOf(path)}
  {@const s = file && summarize(file)}
  <button
    class="flat tw"
    class:on={path === selected}
    class:dim={entry?.excluded}
    type="button"
    title={path}
    style:padding-left="calc(8px + {depth} * var(--w-indent))"
    onclick={() => onselect(path)}
    ondblclick={() => onopen(path)}
    onkeydown={(e) => {
      if (e.key !== "Enter") return;
      e.preventDefault();
      onopen(path);
    }}
    oncontextmenu={(e) => menu(e, path)}
  >
    <span class="chev"></span>
    <span class="name">{name}</span>
    {#if s?.mode}<span class="mono x" title="File mode changed">{s.mode}</span>{/if}
    {#if s}
      <Meter
        width={22}
        segments={[
          { value: s.seenPct, color: "var(--seen)" },
          { value: s.oldPct, color: "var(--old-base)" },
        ]}
      />
    {/if}
    <span class="mono status" style:color={s && STATUS[s.status]}>{s?.status}</span>
  </button>
{/snippet}

<nav aria-label="Files">
  <div class="sh">
    <Segmented
      small
      label="Files shown"
      value={scope}
      onchange={(next: typeof scope) => {
        scope = next;
      }}
      options={[
        { id: "changes", label: "Changes" },
        { id: "all", label: "All files" },
      ]}
    />
    <span class="grow"></span>
    {#if scope === "changes"}<span class="mono count">{paths.length}</span>{/if}
  </div>
  <div class="rows">
    {#if scope === "changes"}
      {#each rows as row (row.path + row.kind)}
        {#if row.kind === "dir"}
          {@render dirRow(row.path, row.name, row.depth, !collapsed.has(row.path), () =>
            toggle(row.path, row.descendants),
          )}
        {:else}
          {@render fileRow(row.path, row.name, row.depth)}
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
    {:else}
      {#each all.rows as row (row.key + row.kind)}
        {#if row.kind === "dir"}
          {@render dirRow(
            row.path,
            row.name,
            row.depth,
            all.expanded.has(row.key),
            () => all.toggle(row.key),
            row.entry,
          )}
        {:else}
          {@render fileRow(row.path, row.name, row.depth, row.entry)}
        {/if}
      {/each}
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
    padding: 0 8px;
    font-size: 12px;
    border-bottom: 1px solid var(--line-soft);
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .count {
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
  .dim {
    color: var(--text-3);
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
  .mark {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--old-base);
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
  .excluded {
    padding-left: 8px;
    color: var(--text-3);
  }
</style>

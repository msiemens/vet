<script lang="ts">
  import { tick, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { DirEntry } from "../../generated/DirEntry";
  import type { FileChange } from "../../generated/FileChange";
  import { bestMatch, type Range } from "../../lib/speedSearch";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import DiffBar from "../../lib/ui/DiffBar.svelte";
  import Marked from "../../lib/ui/Marked.svelte";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import Tab from "../../lib/ui/Tab.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import { AllFiles } from "./AllFiles.svelte";
  import { buildRows, buildTree, filterRows, summarize } from "./tree";

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
    onmenu: (path: string, dir: boolean, x: number, y: number) => void;
  } = $props();

  /** What a row shows of the speed search: whether it is the one Enter opens, and what matched. */
  interface Hit {
    cursor: boolean;
    ranges?: Range[];
  }

  let scope = $state<"changes" | "all">("changes");
  const collapsed = new SvelteSet<string>();
  const unfolded: ReadonlySet<string> = new Set();
  const tree = $derived(buildTree(paths));
  const rows = $derived(buildRows(tree, collapsed));
  // The list function does not change for a mounted tree.
  // svelte-ignore state_referenced_locally
  const all = new AllFiles(list);

  // Typing in the tree filters it to the matches and the folders above them, which show open. A
  // search of "All files" can only filter the folders loaded so far.
  const search = new SpeedSearch();
  const searchedChanges = $derived(
    search.active && scope === "changes" ? filterRows(buildRows(tree, unfolded), search.query) : undefined,
  );
  const searchedAll = $derived(search.active && scope === "all" ? filterRows(all.rows, search.query) : undefined);
  const shownChanges = $derived(searchedChanges?.rows ?? rows);
  const shownAll = $derived(searchedAll?.rows ?? all.rows);
  const hits: ReadonlyMap<object, Range[]> | undefined = $derived((searchedChanges ?? searchedAll)?.hits);
  /** The row Enter opens: an index into the rows shown, -1 outside a search. */
  let cursor = $state(-1);
  let body = $state<HTMLElement>();

  const hasExcluded = $derived(!!excluded && excluded.lines + excluded.items > 0 && !search.active);
  let changesList = $state<VirtualRows>();
  let allList = $state<VirtualRows>();

  const shown = (): readonly { kind: "dir" | "file"; path: string; name: string }[] =>
    scope === "changes" ? shownChanges : shownAll;
  const rowList = () => (scope === "changes" ? changesList : allList);

  function revealPath(path: string) {
    const index = shown().findIndex((r) => r.path === path && r.kind === "file");
    rowList()?.reveal(index);
  }

  // A file selected elsewhere (a tab, the editor) scrolls into view here; rows outside the window are not in the DOM.
  $effect(() => {
    const path = selected;
    const list = scope === "changes" ? changesList : allList;
    if (!path || !list) return;
    untrack(() => revealPath(path));
  });

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

  function menu(e: MouseEvent, path: string, dir: boolean) {
    e.preventDefault();
    onmenu(path, dir, e.clientX, e.clientY);
  }

  function showScope(to: "changes" | "all") {
    search.clear();
    cursor = -1;
    scope = to;
  }

  /** Indices of the rows shown that match, in order. */
  const matches = () => shown().flatMap((r, i) => (hits?.has(r) ? [i] : []));

  /** The row Enter opens is the best match of what is typed. */
  function aim() {
    const found = matches();
    const rows = shown();
    cursor =
      found[
        Math.max(
          0,
          bestMatch(
            search.query,
            found.map((i) => rows[i].name),
          ),
        )
      ] ?? -1;
    rowList()?.reveal(cursor);
  }

  function step(by: number) {
    const found = matches();
    if (!found.length) return;
    const at = found.indexOf(cursor);
    cursor = found[at < 0 ? (by > 0 ? 0 : found.length - 1) : (at + by + found.length) % found.length];
    rowList()?.reveal(cursor);
  }

  function onkeydown(e: KeyboardEvent) {
    if (search.active && !e.altKey && !e.ctrlKey && !e.metaKey) {
      const key = e.key;
      if (key === "ArrowDown" || key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        step(key === "ArrowDown" ? 1 : -1);
        return;
      }
      // A focused row has opened itself.
      const row = key === "Enter" && !e.defaultPrevented ? shown()[cursor] : undefined;
      if (row?.kind === "file") {
        e.preventDefault();
        e.stopPropagation();
        const path = row.path;
        search.clear();
        cursor = -1;
        onopen(path);
        void tick().then(() => revealPath(path));
        return;
      }
    }
    const was = search.active;
    if (!search.type(e)) return;
    if (!search.active) cursor = -1;
    else {
      // The focused row may be filtered away: the list takes the focus, which the next key finds.
      if (!was) body?.focus();
      aim();
    }
  }
</script>

{#snippet dirRow(
  path: string,
  name: string,
  depth: number,
  open: boolean,
  ontoggle: () => void,
  hit: Hit,
  entry?: DirEntry,
)}
  <button
    class="flat tw dir"
    class:dim={entry?.excluded}
    class:cur={hit.cursor}
    type="button"
    title={path}
    style:padding-left="calc(8px + {depth} * var(--w-indent))"
    aria-expanded={open}
    onclick={ontoggle}
    oncontextmenu={(e) => menu(e, path, true)}
  >
    <span class="chev"><Icon name={open ? "open" : "closed"} size={10} /></span>
    <FileIcon {name} folder {open} />
    <span class="name"><Marked text={name} ranges={hit.ranges} /></span>
    {#if entry?.changed}<span class="mark" title="Contains changes"></span>{/if}
  </button>
{/snippet}

{#snippet fileRow(path: string, name: string, depth: number, hit: Hit, entry?: DirEntry)}
  {@const file = fileOf(path)}
  {@const s = file && summarize(file)}
  <button
    class="flat tw"
    class:on={path === selected}
    class:dim={entry?.excluded}
    class:cur={hit.cursor}
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
    oncontextmenu={(e) => menu(e, path, false)}
  >
    <span class="chev"></span>
    <FileIcon {name} />
    <span class="name"><Marked text={name} ranges={hit.ranges} /></span>
    {#if s?.mode}<span class="mono x" title="File mode changed">{s.mode}</span>{/if}
    {#if s && file}
      {@const { added, removed } = file.delta}
      <DiffBar
        width={22}
        {added}
        {removed}
        seen={s.seenPercent}
        old={s.oldBasePercent}
        title="+{added} −{removed}, {Math.round(s.seenPercent + s.oldBasePercent)}% seen"
      />
    {/if}
    <span class="mono status" style:color={s && STATUS[s.status]}>{s?.status}</span>
  </button>
{/snippet}

<nav aria-label="Files">
  <div class="sh">
    <div class="tabs" role="tablist" aria-label="Files shown">
      <Tab selected={scope === "changes"} onselect={() => showScope("changes")}>Changes</Tab>
      <Tab selected={scope === "all"} onselect={() => showScope("all")}>All files</Tab>
    </div>
    <span class="grow"></span>
    {#if scope === "changes"}<span class="mono count">{paths.length}</span>{/if}
  </div>
  <div class="body" role="presentation" tabindex="-1" bind:this={body} {onkeydown} {...search.attributes}>
    {#if search.active}<SpeedSearchField query={search.query} found={!!hits?.size} />{/if}
    {#if scope === "changes"}
      <!-- The excluded group is one more row after the files. -->
      <VirtualRows
        bind:this={changesList}
        count={shownChanges.length + (hasExcluded ? 1 : 0)}
        key={(i) => {
          const r = shownChanges.at(i);
          return r ? r.path + r.kind : "excluded";
        }}
      >
        {#snippet row(i)}
          {@const r = shownChanges.at(i)}
          {#if !r && excluded}
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
          {:else if r?.kind === "dir"}
            {@const hit = { cursor: i === cursor, ranges: hits?.get(r) }}
            {@render dirRow(
              r.path,
              r.name,
              r.depth,
              !!searchedChanges || !collapsed.has(r.path),
              () => toggle(r.path, r.descendants),
              hit,
            )}
          {:else if r}
            {@render fileRow(r.path, r.name, r.depth, { cursor: i === cursor, ranges: hits?.get(r) })}
          {/if}
        {/snippet}
      </VirtualRows>
    {:else}
      <VirtualRows
        bind:this={allList}
        count={shownAll.length}
        key={(i) => {
          const r = shownAll.at(i);
          return r ? r.key + r.kind : "";
        }}
      >
        {#snippet row(i)}
          {@const r = shownAll.at(i)}
          {#if r?.kind === "dir"}
            {@const hit = { cursor: i === cursor, ranges: hits?.get(r) }}
            {@render dirRow(r.path, r.name, r.depth, all.expanded.has(r.key), () => all.toggle(r.key), hit, r.entry)}
          {:else if r}
            {@render fileRow(r.path, r.name, r.depth, { cursor: i === cursor, ranges: hits?.get(r) }, r.entry)}
          {/if}
        {/snippet}
      </VirtualRows>
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
  /* Tabs of the header itself, like the editor's tab strip next to it. */
  .sh {
    display: flex;
    align-items: center;
    height: var(--h-tab);
    padding-right: 8px;
    font-size: 12px;
    border-bottom: 1px solid var(--line);
    flex: none;
  }
  .tabs {
    display: flex;
    align-self: stretch;
  }
  .grow {
    flex: 1;
  }
  .count {
    color: var(--text-3);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
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
  /* The row Enter opens while searching. */
  .cur {
    background: var(--select);
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

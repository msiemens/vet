<script lang="ts">
  import { tick, untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import type { DirEntry } from "../../generated/DirEntry";
  import type { ExcludedGroup } from "../../generated/ExcludedGroup";
  import type { ReviewedFile } from "../../state/reviewModel";
  import { dirname } from "../../lib/paths";
  import { letterOfChange } from "../../lib/changeLetter";
  import { arrowKey, toggleIn } from "../../lib/foldKeys";
  import { plainKey } from "../../lib/listKeys";
  import { aimIndex, stepMatch, type Range } from "../../lib/speedSearch";
  import { fullName } from "../../lib/symbolLabel";
  import ChangeLetter from "../../lib/ui/ChangeLetter.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import SeverityIcon from "../../lib/ui/SeverityIcon.svelte";
  import DiffBar from "../../lib/ui/DiffBar.svelte";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import { deltaText } from "../../lib/ui/lineDelta";
  import { ListNav } from "../../lib/ui/listNav";
  import Marked from "../../lib/ui/Marked.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import ProgressBar from "../../lib/ui/ProgressBar.svelte";
  import type { CommitUnit } from "../../generated/CommitUnit";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import TreeRow from "../../lib/ui/TreeRow.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import { commands } from "../../state/commands";
  import { keyOf } from "../../state/symbolKey";
  import { symbols } from "../../state/symbols.svelte";
  import { SymbolGlyph } from "../symbols";
  import { AllFiles } from "./AllFiles.svelte";
  import type { AllRow } from "./allFilesRows";
  import CommitList from "./CommitList.svelte";
  import { excludedRows, extraRows } from "./excludedRows";
  import NarrowingChip from "./NarrowingChip.svelte";
  import { closedKey, fileSymbolRows, type SymbolRow } from "./symbolRows";
  import { type Row, buildRows, buildTree, filterRows, summarize } from "./tree";

  let {
    tool,
    paths,
    fileOf,
    revision,
    list,
    selected,
    excluded,
    excludedOpen = $bindable(false),
    narrowed,
    onreset,
    none,
    changes,
    commits = [],
    commit,
    oncommit,
    onselect,
    onopen,
    onopensymbol,
    onmenu,
  }: {
    /** Which tool window this is: the changed files, every file, or the commits with the files of the one on view. */
    tool: "changes" | "all" | "commits";
    /** Changes only with the changeset's revision, so the tree is built once per revision. */
    paths: readonly string[];
    /** The number after "Changes": how many files the changeset changes. */
    changes: number;
    /** The commits with their progress, while the changeset is reviewed by commit. */
    commits?: CommitUnit[];
    /** The index of the commit on view, by commit. */
    commit?: number;
    /** Shows the commit at this index of `commits`. */
    oncommit: (index: number) => void;
    /** A file's current state; reading it subscribes to that file alone. */
    fileOf: (path: string) => ReviewedFile | undefined;
    /** The manifest revision; "All files" reloads when it moves. */
    revision?: number;
    /** The children of a directory of the target snapshot, for "All files". */
    list: (path: string) => Promise<{ revision: number; entries: DirEntry[] } | undefined>;
    selected?: string;
    /** Changes hidden by the review filter, collapsed into one group that opens to the files and why each is left out. */
    excluded?: { lines: number; items: number; groups: readonly ExcludedGroup[] };
    /** The group of excluded files is open. */
    excludedOpen?: boolean;
    /** While the review is narrowed: what it is narrowed to, as the chip words it. */
    narrowed?: string;
    /** Ends the narrowing. */
    onreset: () => void;
    /** The line under the header while the Changes tool lists no file. */
    none: string;
    /** A single click, once no second click follows. */
    onselect: (path: string) => void;
    /** Double click or Enter. */
    onopen: (path: string) => void;
    /** Double click, Enter or F4 on a changed symbol: its definition (the base side's for a removed one). */
    onopensymbol: (row: SymbolRow) => void;
    /** Context menu on a file or folder row, at the pointer position. */
    onmenu: (path: string, dir: boolean, x: number, y: number) => void;
  } = $props();

  const TITLES = { changes: "Changes", all: "All Files", commits: "Commits" } as const;

  /** What a row shows of the speed search: whether it is the one Enter opens, and what matched. */
  interface Hit {
    cursor: boolean;
    ranges?: Range[];
  }

  const scope = $derived(tool === "all" ? "all" : "changes");

  // A search belongs to the window it was typed in.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the tool a dependency
    tool;
    untrack(() => {
      search.clear();
      cursor = -1;
    });
  });

  // A click on a file marks it at once; what it shows waits for a second click that would open the file instead.
  let pressed = $state<string>();
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the selection a dependency
    selected;
    untrack(() => (pressed = undefined));
  });
  // The highlighted row: an index into the rows shown, -1 for none. While searching it is the row Enter opens.
  let cursor = $state(-1);
  /**
   * A single click or Space: a file's place in the review, a changed symbol on show in the panel, a
   * folder or a holder of symbols open or shut.
   */
  function pick(row: Row | AllRow) {
    if (row.kind === "file") onselect(row.path);
    else if (row.kind === "symbol" && row.change !== undefined) symbols.choose(row.symbol, row.change);
    else toggleRow(row);
  }
  /** A double click or Enter: the file in a tab of its own, a changed symbol's definition; a folder opens or shuts. */
  function openRow(row: Row | AllRow) {
    if (row.kind === "file") onopen(row.path);
    else if (row.kind === "symbol" && row.change !== undefined) onopensymbol(row);
    else toggleRow(row);
  }
  let body = $state<HTMLElement>();
  const rowNav = new ListNav<Row | AllRow>({
    element: () => body,
    items: () => shown(),
    single: pick,
    double: openRow,
    highlight: { get: () => cursor, set: (i) => (cursor = i) },
    reveal: (i) => void rowList()?.reveal(i),
  });
  $effect(() => () => rowNav.cancel());

  /** Whether a click was on the chevron, which opens or shuts what is below the row without choosing it. */
  const onChevron = (e: MouseEvent) => e.target instanceof Element && !!e.target.closest(".chev");

  function clickFile(e: MouseEvent, path: string, index: number) {
    if (onChevron(e) && hasSymbols(path)) {
      cursor = index;
      body?.focus({ preventScroll: true });
      toggleFile(path);
      return;
    }
    if (e.detail !== 0) pressed = path;
    rowNav.click(e, index);
  }

  function clickSymbol(e: MouseEvent, row: SymbolRow, index: number) {
    if (onChevron(e) && row.holds) {
      cursor = index;
      body?.focus({ preventScroll: true });
      toggleSymbol(row);
      return;
    }
    rowNav.click(e, index);
  }

  function clickDir(index: number, ontoggle: () => void) {
    cursor = index;
    body?.focus({ preventScroll: true });
    ontoggle();
  }

  function toggleRow(row: Row | AllRow) {
    if (row.kind === "symbol") toggleSymbol(row);
    else if ("key" in row) all.toggle(row.key);
    else if (row.kind === "dir") toggle(row.path, row.descendants);
  }

  /** Where `path` is in the tree: its folders open, the row scrolled into view. */
  export async function reveal() {
    const path = selected;
    if (!path) return;
    search.clear();
    cursor = -1;
    const folders: string[] = [];
    for (let dir = dirname(path); dir; dir = dirname(dir)) folders.unshift(dir);
    if (scope === "changes") for (const dir of folders) collapsed.delete(dir);
    else await all.expandTo(folders);
    await tick();
    revealPath(path);
  }

  /** The height of the commit list above the files of the commit, which the divider moves. */
  let listHeight = $state(180);
  const MIN_LIST = 60;
  const MIN_TREE = 80;
  let nav = $state<HTMLElement>();

  function dragDivider(e: PointerEvent) {
    const start = { y: e.clientY, height: listHeight };
    const room = () => (nav?.clientHeight ?? 400) - MIN_TREE;
    const move = (m: PointerEvent) => {
      listHeight = Math.max(MIN_LIST, Math.min(room(), start.height + m.clientY - start.y));
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    e.preventDefault();
  }

  function nudgeDivider(e: KeyboardEvent) {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    listHeight = Math.max(MIN_LIST, listHeight + (e.key === "ArrowDown" ? 16 : -16));
  }

  const collapsed = new SvelteSet<string>();
  const unfolded: ReadonlySet<string> = new Set();
  const closedNone: ReadonlySet<string> = new Set();
  const tree = $derived(buildTree(paths));

  // The files show their symbols all or none, as the header's button last chose; a file the viewer
  // opened or shut since follows that, and so does a file the changeset gains later.
  let openAll = $state(false);
  const fileOpen = new SvelteMap<string, boolean>();
  const isOpen = (path: string) => fileOpen.get(path) ?? openAll;
  /** The holders of symbols that are shut, by `closedKey`. */
  const closedSymbols = new SvelteSet<string>();

  const loaded = (path: string) => {
    const file = symbols.files.get(path);
    return file?.state === "done" ? file : undefined;
  };
  const hasSymbols = (path: string) => !!loaded(path)?.symbols.length;
  const symbolsOf = (open: (path: string) => boolean, closed: ReadonlySet<string>) =>
    tool === "changes"
      ? (path: string, depth: number) => {
          const file = open(path) ? loaded(path) : undefined;
          return file ? fileSymbolRows(path, file.symbols, file.context, depth, closed) : [];
        }
      : undefined;

  const rows = $derived(buildRows(tree, collapsed, symbolsOf(isOpen, closedSymbols)));
  const foldable = $derived(tool === "changes" ? paths.filter(hasSymbols) : []);
  const allOpen = $derived(foldable.length > 0 && foldable.every(isOpen));

  const toggleFile = (path: string) => fileOpen.set(path, !isOpen(path));
  const toggleSymbol = (row: SymbolRow) => toggleIn(closedSymbols, closedKey(row.path, row.key));
  /** Opens every file that has symbols and every holder in them, or shuts the files. */
  function toggleAll() {
    openAll = !allOpen;
    fileOpen.clear();
    if (openAll) closedSymbols.clear();
  }
  // The list function does not change for a mounted tree.
  // svelte-ignore state_referenced_locally
  const all = new AllFiles(list);

  // Typing in the tree filters it to the matches and the folders above them, which show open. A
  // search of "All files" can only filter the folders loaded so far.
  const search = new SpeedSearch();
  const searchedChanges = $derived(
    search.active && scope === "changes"
      ? filterRows(
          buildRows(
            tree,
            unfolded,
            symbolsOf(() => true, closedNone),
          ),
          search.query,
        )
      : undefined,
  );
  const searchedAll = $derived(search.active && scope === "all" ? filterRows(all.rows, search.query) : undefined);
  const shownChanges = $derived(searchedChanges?.rows ?? rows);
  const shownAll = $derived(searchedAll?.rows ?? all.rows);
  const hits: ReadonlyMap<object, Range[]> | undefined = $derived((searchedChanges ?? searchedAll)?.hits);

  const excludedFiles = $derived(excluded?.groups.reduce((sum, g) => sum + g.count, 0) ?? 0);
  const hasExcluded = $derived(excludedFiles > 0 && !search.active);
  const heldRows = $derived(excludedRows(excluded?.groups ?? []));
  const extra = $derived(extraRows(hasExcluded, excludedOpen, heldRows));
  let changesList = $state<VirtualRows>();
  let allList = $state<VirtualRows>();

  const shown = (): readonly (Row | AllRow)[] => (scope === "changes" ? shownChanges : shownAll);
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

  // The cursor went into a changed symbol: its file opens and its row comes into view. The tree keeps its focus.
  $effect(() => {
    const request = symbols.reveal;
    // Also when the file's symbols arrive after the cursor was in one.
    const arrived = request && loaded(request.path);
    if (request && arrived && tool === "changes" && request.n !== revealed)
      untrack(() => void revealSymbol(request.path, keyOf(request.symbol.id), request.n));
  });
  /** The last request of the cursor that was answered, whether or not its row was found. */
  let revealed = 0;

  async function revealSymbol(path: string, id: string, n: number) {
    revealed = n;
    search.clear();
    cursor = -1;
    for (let dir = dirname(path); dir; dir = dirname(dir)) collapsed.delete(dir);
    fileOpen.set(path, true);
    for (const key of [...closedSymbols]) if (key.startsWith(`${path}\0`)) closedSymbols.delete(key);
    await tick();
    const index = shown().findIndex((r) => r.kind === "symbol" && r.change !== undefined && keyOf(r.symbol.id) === id);
    if (index < 0) return;
    rowList()?.reveal(index);
  }

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

  /** Indices of the rows shown that match, in order. */
  const matches = () => shown().flatMap((r, i) => (hits?.has(r) ? [i] : []));

  /** The row Enter opens is the best match of what is typed. */
  function aim() {
    const rows = shown();
    cursor = aimIndex(search.query, matches(), (i) => rows[i].name);
    rowList()?.reveal(cursor);
  }

  function step(by: number) {
    const to = stepMatch(matches(), cursor, by);
    if (to === undefined) return;
    cursor = to;
    rowList()?.reveal(cursor);
  }

  /** ← and → open, shut and move through files with symbols and symbols, F4 opens a symbol; answers whether the key was one. */
  function changesKey(e: KeyboardEvent): boolean {
    if (cursor < 0 || cursor >= shownChanges.length) return false;
    const row = shownChanges[cursor];
    if (e.key === "F4") {
      if (row.kind !== "symbol" || row.change === undefined) return false;
      onopensymbol(row);
      return true;
    }
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return false;
    const step = arrowKey(shownChanges, cursor, e.key === "ArrowLeft" ? "left" : "right", {
      holds: (r) => (r.kind === "symbol" ? r.holds : r.kind === "file" && hasSymbols(r.path)),
      open: (r) => (r.kind === "symbol" ? !closedSymbols.has(closedKey(r.path, r.key)) : isOpen(r.path)),
      leaves: (r) => r.kind === "symbol",
    });
    if (!step) return false;
    if ("move" in step) {
      cursor = step.move;
      rowList()?.reveal(cursor);
    } else if (step.toggle.kind === "symbol") toggleSymbol(step.toggle);
    else toggleFile(step.toggle.path);
    return true;
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
    if (!search.active && tool === "changes" && plainKey(e) && changesKey(e)) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    const was = search.active;
    if (!search.type(e)) {
      if (!search.active && rowNav.keydown(e)) {
        e.preventDefault();
        e.stopPropagation();
      }
      return;
    }
    if (!search.active) cursor = -1;
    else {
      // The focused row may be filtered away: the list takes the focus, which the next key finds.
      if (!was) body?.focus();
      aim();
    }
  }
</script>

{#snippet dirRow(
  index: number,
  path: string,
  name: string,
  depth: number,
  open: boolean,
  ontoggle: () => void,
  hit: Hit,
  entry?: DirEntry,
)}
  <button
    class="flat pick tw dir"
    class:dim={entry?.excluded}
    class:cur={hit.cursor}
    type="button"
    tabindex="-1"
    title={path}
    style:padding-left="calc(8px + {depth} * var(--w-indent))"
    aria-expanded={open}
    onclick={() => clickDir(index, ontoggle)}
    oncontextmenu={(e) => menu(e, path, true)}
  >
    <span class="chev"><Icon name={open ? "open" : "closed"} size={10} /></span>
    <FileIcon {name} folder {open} />
    <span class="name chain"><PathText path={name} title={path} ranges={hit.ranges} /></span>
    {#if entry?.changed}<span class="mark" title="Contains changes"></span>{/if}
  </button>
{/snippet}

{#snippet fileRow(index: number, path: string, name: string, depth: number, hit: Hit, entry?: DirEntry)}
  {@const file = fileOf(path)}
  {@const s = file && summarize(file)}
  {@const flagged = file?.completion.flaggedHunks ?? 0}
  {@const analysis = tool === "changes" ? symbols.files.get(path) : undefined}
  {@const open = search.active || isOpen(path)}
  <button
    class="flat pick tw"
    class:on={path === (pressed ?? selected)}
    class:dim={entry?.excluded}
    class:cur={hit.cursor}
    class:flagged
    type="button"
    tabindex="-1"
    title={flagged ? `${path}\n${flagged} flagged ${flagged === 1 ? "hunk" : "hunks"}` : path}
    style:padding-left="calc(8px + {depth} * var(--w-indent))"
    onclick={(e) => clickFile(e, path, index)}
    oncontextmenu={(e) => menu(e, path, false)}
  >
    <span class="chev">
      {#if analysis?.state === "done" && analysis.symbols.length}<Icon name={open ? "open" : "closed"} size={10} />{/if}
    </span>
    <FileIcon {name} />
    <span class="name"><Marked text={name} ranges={hit.ranges} /></span>
    {#if analysis?.state === "loading"}
      <Spinner label="Analysing the symbols" />
    {:else if analysis?.state === "failed"}
      <span class="warn" title={analysis.message}><SeverityIcon severity="warning" /></span>
    {:else if analysis?.state === "done" && analysis.symbols.length && !open}
      {@const n = analysis.symbols.length}
      <span class="mono x" title="{n} changed {n === 1 ? 'symbol' : 'symbols'}">{n}</span>
    {/if}
    {#if s?.mode}<span class="mono x" title="File mode changed">{s.mode}</span>{/if}
    {#if s && file}
      {@const { added, removed } = file.delta}
      <DiffBar
        width={22}
        {added}
        {removed}
        seen={s.seenPercent}
        old={s.oldBasePercent}
        title="{deltaText(added, removed)}, {Math.round(s.seenPercent)}% seen"
      />
    {/if}
    <ChangeLetter letter={s?.status} />
  </button>
{/snippet}

{#snippet symbolRow(index: number, r: SymbolRow, hit: Hit)}
  {@const changed = r.change !== undefined}
  {@const shut = !search.active && closedSymbols.has(closedKey(r.path, r.key))}
  <TreeRow
    depth={r.depth}
    open={r.holds ? !shut : undefined}
    tone={changed ? "strong" : "soft"}
    mono
    gone={r.change === "removed"}
    on={changed && !!symbols.selected && keyOf(symbols.selected.symbol.id) === keyOf(r.symbol.id)}
    cur={hit.cursor}
    title={fullName(r.symbol.id)}
    status={r.change && letterOfChange(r.change)}
    statusTitle={r.change}
    tabindex={-1}
    onclick={(e) => clickSymbol(e, r, index)}
  >
    {#snippet glyph()}<SymbolGlyph kind={r.symbol.kind} />{/snippet}
    <Marked text={r.name} ranges={hit.ranges} />
  </TreeRow>
{/snippet}

<nav aria-label={TITLES[tool]} bind:this={nav}>
  <div class="sh sh-bar">
    <span>{TITLES[tool]}</span>
    {#if tool === "commits"}<span class="mono n">{commits.length}</span>{/if}
    {#if tool === "changes" && narrowed}<NarrowingChip label={narrowed} {onreset} />{/if}
    <span class="grow"></span>
    {#if tool === "changes"}
      {#if symbols.progress.done < symbols.progress.total}
        <span class="mono n" title="Files whose changed symbols are found, of the files to analyse"
          >symbols {symbols.progress.done}/{symbols.progress.total}</span
        >
      {/if}
      <IconButton
        icon={allOpen ? "collapseAll" : "expandAll"}
        label={allOpen ? "Collapse all" : "Expand all symbols"}
        disabled={!foldable.length}
        size={22}
        iconSize={14}
        onclick={toggleAll}
      />
      <span class="mono n end">{changes}</span>
    {:else if tool === "all"}
      <IconButton
        icon="locate"
        label={commands.label("view.revealFile")}
        title={commands.hint("view.revealFile")}
        disabled={!commands.isEnabled("view.revealFile")}
        size={22}
        iconSize={13}
        onclick={() => commands.run("view.revealFile")}
      />
    {/if}
  </div>
  {#if tool === "changes" && symbols.progress.done < symbols.progress.total}
    <div class="progress">
      <ProgressBar fraction={symbols.progress.done / symbols.progress.total} label="Symbols found" />
    </div>
  {/if}
  {#if tool === "commits"}
    <div class="commits" style:height="{listHeight}px">
      {#if commits.length}
        <CommitList {commits} selected={commit} onselect={oncommit} />
      {:else}
        <EmptyState align="start" text="Loading the commits…" />
      {/if}
    </div>
    <!-- A focusable separator is a widget (WAI-ARIA window splitter). -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="divider"
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize the commit list"
      tabindex="0"
      onpointerdown={dragDivider}
      onkeydown={nudgeDivider}
    ></div>
  {/if}
  <!-- The list takes the keys of its rows, which are not in the tab order. -->
  <div class="body" role="presentation" bind:this={body} {onkeydown} {...rowNav.attributes} {...search.attributes}>
    {#if search.active}<SpeedSearchField query={search.query} found={!!hits?.size} />{/if}
    {#if scope === "changes"}
      {#if !paths.length && !search.active}<EmptyState align="start" text={none} />{/if}
      <!-- The excluded group is one more row after the files, and the files it holds follow it while it is open. -->
      <VirtualRows
        bind:this={changesList}
        count={shownChanges.length + extra}
        key={(i) => {
          const r = shownChanges.at(i);
          if (!r) return `excluded${i - shownChanges.length}`;
          return r.kind === "symbol" ? closedKey(r.path, r.key) + r.kind : r.path + r.kind;
        }}
      >
        {#snippet row(i)}
          {@const r = shownChanges.at(i)}
          {@const held = heldRows.at(i - shownChanges.length - 1)}
          {#if !r && excluded && i === shownChanges.length}
            <button
              class="flat tw excluded"
              type="button"
              tabindex="-1"
              aria-expanded={excludedOpen}
              title="{excluded.lines} changed lines and {excluded.items} other changes excluded by the review filter"
              onclick={() => (excludedOpen = !excludedOpen)}
            >
              <span class="chev"><Icon name={excludedOpen ? "open" : "closed"} size={10} /></span><Icon
                name="filter"
                size={11}
                stroke={2}
              />Excluded<span class="mono x">{excludedFiles}</span>
            </button>
          {:else if !r && held?.path !== undefined}
            {@const path = held.path}
            <button
              class="flat tw held"
              type="button"
              tabindex="-1"
              title="{held.name}: {held.why}"
              onclick={() => onopen(path)}
            >
              <span class="name">{held.name}</span>
              <span class="why">{held.why}</span>
            </button>
          {:else if !r && held}
            <div class="tw held" title="{held.name}: {held.why}">
              <span class="name">{held.name}</span>
              <span class="why">{held.why}</span>
            </div>
          {:else if r?.kind === "dir"}
            {@const hit = { cursor: i === cursor, ranges: hits?.get(r) }}
            {@render dirRow(
              i,
              r.path,
              r.name,
              r.depth,
              !!searchedChanges || !collapsed.has(r.path),
              () => toggle(r.path, r.descendants),
              hit,
            )}
          {:else if r?.kind === "symbol"}
            {@render symbolRow(i, r, { cursor: i === cursor, ranges: hits?.get(r) })}
          {:else if r}
            {@render fileRow(i, r.path, r.name, r.depth, { cursor: i === cursor, ranges: hits?.get(r) })}
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
            {@render dirRow(i, r.path, r.name, r.depth, all.expanded.has(r.key), () => all.toggle(r.key), hit, r.entry)}
          {:else if r}
            {@render fileRow(i, r.path, r.name, r.depth, { cursor: i === cursor, ranges: hits?.get(r) }, r.entry)}
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
  /* The commit list above the files of the commit selected, its height set by the divider. */
  .commits {
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* Text at the end of the bar keeps the inset of the title, which the bar's padding sets for a button. */
  .end {
    margin-right: 4px;
  }
  .divider {
    flex: none;
    height: 5px;
    margin: -2px 0;
    position: relative;
    z-index: 1;
    cursor: row-resize;
    border-top: 1px solid var(--line);
    box-sizing: content-box;
  }
  .divider:hover,
  .divider:focus-visible {
    background: var(--toggle-press);
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
  .flagged .name {
    color: var(--warn);
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
  .chain {
    display: flex;
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
  .warn {
    flex: none;
    display: inline-flex;
  }
  .progress {
    flex: none;
  }
  .excluded {
    padding-left: 8px;
    color: var(--text-3);
  }
  .held {
    padding-left: calc(8px + var(--w-chev) + 6px);
    color: var(--text-3);
  }
  .why {
    flex: none;
    max-width: 62%;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 11px;
  }
</style>

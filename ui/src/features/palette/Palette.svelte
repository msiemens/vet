<script lang="ts">
  // Search Everywhere: one popup under the header for names (files, symbols, changesets, comment
  // threads, actions), with a tab for each kind. Arrow keys move, Tab or Ctrl+Tab changes the tab, Enter chooses,
  // Shift+Enter shows a changed file or symbol in All changes, Escape closes.
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { bindingOf, matches } from "../../lib/keymap";
  import { splitPath } from "../../lib/paths";
  import { highlightRuns, sliceRanges, type Range } from "../../lib/speedSearch";
  import DiffBar from "../../lib/ui/DiffBar.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import { changeset } from "../../state/changeset.svelte";
  import { keyId } from "../../state/changesetKey";
  import { commands } from "../../state/commands";
  import { isMac } from "../../state/host";
  import { progress } from "../../state/progress.svelte";
  import { queue } from "../../state/queue.svelte";
  import { serverList } from "../../state/serverList.svelte";
  import { threads } from "../../state/threads.svelte";
  import { symbols } from "../../state/symbols.svelte";
  import { queueRows, labelOf, reviewedOf, sourceText } from "../queue";
  import { statusLetter } from "../review";
  import { SymbolGlyph } from "../symbols";
  import type { Choice } from "./choice";
  import { tabStep } from "./tabKey";
  import { FIND_IN_FILES, PALETTE_COMMANDS } from "./paletteCommands";
  import { palette } from "./paletteState.svelte";
  import {
    type Candidates,
    type Entry,
    type PaletteTab,
    type Row,
    TABS,
    TITLES,
    build,
    entriesOf,
    rank,
    recentId,
  } from "./paletteRows";
  import {
    actionCandidates,
    changesetCandidates,
    commandEntries,
    failedServers,
    fileCandidates,
    indexingNotes,
    serverSymbolCandidates,
    symbolCandidates,
    threadCandidates,
    unchangedCandidates,
  } from "./paletteSources";

  let { onchoose, onsettings }: { onchoose: (choice: Choice) => void; onsettings: () => void } = $props();

  /** How far the Page keys move. */
  const PAGE = 8;
  /** How long the query rests before the language servers are asked. */
  const SERVER_PAUSE_MS = 120;
  const LETTER_COLORS = { M: "var(--old-base)", A: "var(--add-text)", D: "var(--del-text)", R: "var(--text-2)" };
  /** The tabs the Unchanged files toggle applies to. */
  const TOGGLES: readonly PaletteTab[] = ["all", "file", "symbol"];
  const SYMBOL_TABS: readonly PaletteTab[] = ["all", "symbol"];
  const SYMBOL_LETTERS = { modified: "M", added: "A", removed: "D" } as const;
  const NEXT_TAB = bindingOf("Ctrl-Tab", isMac ? "Meta" : "Ctrl").label;

  let cursor = $state(0);
  let root = $state<HTMLElement>();
  let list = $state<HTMLElement>();
  let input = $state<HTMLInputElement>();
  let before: Element | null = null;
  // A click that began before the popup was there is not one outside it: the button that opens it.
  let shownAt = 0;
  // What was clicked outside took the focus itself.
  let clickedAway = false;

  const files = $derived(
    fileCandidates(
      (changeset.layout?.files ?? []).map((f) => {
        const completion = changeset.reviewed(f.path)?.completion;
        return {
          path: f.path,
          status: statusLetter(f),
          added: f.delta.added,
          removed: f.delta.removed,
          seen: completion?.seenPercent ?? 0,
          oldBase: completion?.oldBasePercent ?? 0,
        };
      }),
    ),
  );
  const symbolList = $derived(
    symbolCandidates(
      [...symbols.files].flatMap(([path, load]) =>
        load.state === "done"
          ? [{ symbols: load.symbols, seen: changeset.reviewed(path)?.completion.seenPercent ?? 0 }]
          : [],
      ),
    ),
  );
  const onViewId = $derived(queue.onView && keyId(queue.onView));
  const changesets = $derived(
    changesetCandidates(
      queueRows(queue.local, queue.pullRequests).map((row) => {
        const id = keyId(row.key);
        const { number, text } = labelOf(row);
        const reviewed = reviewedOf(row.review, id === onViewId ? queue.onViewFrame?.facts.completion : undefined);
        return {
          id,
          number,
          title: text,
          source: sourceText(row),
          github: row.key.type === "pullRequest",
          onView: id === onViewId,
          progress:
            reviewed.kind === "progress"
              ? { kind: "progress", seen: reviewed.seen }
              : reviewed.kind === "finished"
                ? { kind: "finished" }
                : reviewed,
        };
      }),
    ),
  );
  const actions = $derived(actionCandidates(commandEntries(commands, palette.context)));
  /** The files the changeset does not change, once the list of all of them is here. */
  const others = $derived(palette.files ? unchangedCandidates(palette.files.paths, new Set(changeset.paths)) : []);
  const typed = $derived(palette.query.trim());
  const revision = $derived(changeset.layout?.shown.revision);
  /** The servers are asked for symbols by name when unchanged files are searched and symbols are listed. */
  const asksServers = $derived(
    palette.unchanged && typed !== "" && revision !== undefined && SYMBOL_TABS.includes(palette.tab),
  );
  /** What the servers found for the query as it stands, on the revision on view. */
  const answer = $derived(
    palette.served?.query === typed && palette.served.revision === revision ? palette.served : undefined,
  );
  const found = $derived(answer?.languages ?? []);
  /** The changed files whose changed symbols are not known: the servers' symbols in them could be changed ones. */
  const unsettled = $derived(new Set(changeset.paths.filter((p) => symbols.files.get(p)?.state !== "done")));
  const candidates: Candidates = $derived({
    file: palette.unchanged ? [...files, ...others] : files,
    symbol: [...symbolList, ...serverSymbolCandidates(found, symbolList, unsettled)],
    changeset: changesets,
    thread: threadCandidates(threads.items),
    action: actions,
  });

  const built = $derived(
    build(palette.tab, palette.query, candidates, palette.recent, {
      files: palette.unchanged && palette.listing && !palette.files,
      symbols: asksServers && !answer,
    }),
  );
  const failed = $derived(failedServers(found, serverList.servers));
  const indexing = $derived(indexingNotes(found, serverList.servers, progress.tasks));
  const entries = $derived(entriesOf(built.groups));
  const at = $derived(Math.min(cursor, entries.length - 1));
  const current = $derived<Entry | undefined>(entries[at]);
  /** The index of each group's first entry among all entries. */
  const starts = $derived(
    built.groups.reduce<number[]>((acc, g, i) => [...acc, (acc[i] ?? 0) + g.entries.length], [0]),
  );
  /** The commit a view pinned to one opens files at. */
  const pinned = $derived(
    changeset.shownCommit === undefined ? undefined : changeset.commits[changeset.shownCommit]?.commit.shortId,
  );
  /** How many unchanged files match what was typed, when nothing else does and they are not searched. */
  const hidden = $derived(
    !palette.unchanged && typed && !entries.length && TOGGLES.includes(palette.tab)
      ? rank(others, typed, []).length
      : 0,
  );
  // The list of all files is asked for when they are searched, or when it can say what a search found nothing of.
  $effect(() => {
    const wanted = palette.unchanged || (typed !== "" && !entries.length);
    if (!wanted || palette.files?.revision === changeset.layout?.shown.revision) return;
    untrack(() => {
      if (!palette.listing) void palette.listFiles();
    });
  });
  // A pause in typing asks the servers; what is typed next cancels the ask.
  $effect(() => {
    if (!asksServers || revision === undefined) {
      untrack(() => palette.forgetSymbols());
      return;
    }
    const query = typed;
    const at = revision;
    const timer = setTimeout(() => palette.searchSymbols(query, at), SERVER_PAUSE_MS);
    return () => {
      clearTimeout(timer);
      palette.cancelSymbols();
    };
  });
  const findOff = $derived(commands.disabledReason(FIND_IN_FILES) ?? "");
  const hint = $derived(
    current?.type === "row" && current.row.item.kind === "action" && current.row.item.off
      ? current.row.item.disabled
      : current?.type === "find"
        ? findOff
        : undefined,
  );

  // What was typed or chosen as the tab starts the list over.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the reads make them dependencies
    (palette.query, palette.tab);
    cursor = 0;
  });

  // The changesets are listed afresh each time the popup opens, as the switcher's popover does.
  $effect(() => {
    void queue.loadAll();
    return queue.watch();
  });

  onMount(() => {
    shownAt = performance.now();
    before = document.activeElement;
    input?.focus();
  });
  // The focus goes back to where it was, unless a click went elsewhere; a choice that moves it does so after.
  onDestroy(() => {
    if (!clickedAway && before instanceof HTMLElement && before.isConnected) before.focus({ preventScroll: true });
  });

  async function move(to: number) {
    cursor = Math.max(0, Math.min(entries.length - 1, to));
    await tick();
    list?.querySelector(`[data-at="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function step(delta: number) {
    const ids = TABS.map((t) => t.id);
    palette.tab = ids[(ids.indexOf(palette.tab) + delta + ids.length) % ids.length];
  }

  function choose(entry: Entry | undefined, shift: boolean) {
    if (!entry) return;
    if (entry.type === "more") {
      palette.tab = entry.kind;
      input?.focus();
      return;
    }
    if (entry.type === "find") {
      if (!commands.isEnabled(FIND_IN_FILES)) return;
      finish(() => void commands.run(FIND_IN_FILES));
      return;
    }
    const { item, line } = entry.row;
    if (item.kind === "action" && item.off) return;
    palette.choose(recentId(item.kind, item.key));
    switch (item.kind) {
      case "file":
        finish(() =>
          onchoose({
            type: "file",
            path: item.path,
            line: line === undefined ? undefined : line - 1,
            inChanges: shift && item.changed,
          }),
        );
        break;
      case "symbol":
        finish(() =>
          onchoose({ type: "symbol", symbol: item.info, change: item.change, reveal: shift && !!item.change }),
        );
        break;
      case "changeset":
        finish(() => onchoose({ type: "changeset", id: item.key }));
        break;
      case "thread":
        finish(() => onchoose({ type: "thread", id: Number(item.key) }));
        break;
      case "action":
        finish(() => onchoose({ type: "action", id: item.key }));
        break;
    }
  }

  /** Closes the popup and, once the focus is back where it was, runs what was chosen. */
  function finish(run: () => void) {
    palette.close();
    void tick().then(run);
  }

  // A list that scrolls under a pointer at rest makes the browser repeat the pointer's last move
  // with no movement; only a pointer that moved takes the highlight.
  function hover(e: MouseEvent, index: number) {
    if (e.movementX !== 0 || e.movementY !== 0) cursor = index;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.isComposing) return;
    const take = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    // The chords that open the popup move it to their tab; the registry cannot match an Option chord inside a field.
    for (const id of PALETTE_COMMANDS) {
      const spec = commands.binding(id)?.spec;
      if (spec && matches(spec, e, true)) {
        take();
        void commands.run(id);
        return;
      }
    }
    const tabs = tabStep(e);
    if (tabs) {
      take();
      step(tabs);
      return;
    }
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    switch (e.key) {
      case "Escape":
        take();
        palette.close();
        break;
      case "ArrowDown":
        take();
        void move(at + 1);
        break;
      case "ArrowUp":
        take();
        void move(at - 1);
        break;
      case "PageDown":
        take();
        void move(at + PAGE);
        break;
      case "PageUp":
        take();
        void move(at - PAGE);
        break;
      case "Enter":
        take();
        choose(current, e.shiftKey);
        break;
    }
  }

  /** The last two parts of a path, which is all a row has room for. */
  function shortPath(path: string): string {
    return path.split("/").slice(-2).join("/");
  }
</script>

<svelte:window
  onclick={(e) => {
    if (e.timeStamp < shownAt || !root || root.contains(e.target as Node)) return;
    clickedAway = true;
    palette.close();
  }}
/>

{#snippet marked(text: string, ranges: readonly Range[] | undefined)}
  {#each highlightRuns(text, ranges ?? []) as run, i (i)}{#if run.hit}<mark>{run.text}</mark
      >{:else}{run.text}{/if}{/each}
{/snippet}

{#snippet split(lead: string, last: string, ranges: readonly Range[], title: string)}
  <span class="path-text" {title}
    ><span class="dir">{@render marked(lead, sliceRanges(ranges, 0, lead.length))}</span><span class="name"
      >{@render marked(last, sliceRanges(ranges, lead.length, lead.length + last.length))}</span
    ></span
  >
{/snippet}

{#snippet letter(char: string)}
  <span class="ltr" style:color={LETTER_COLORS[char as keyof typeof LETTER_COLORS]}>{char}</span>
{/snippet}

{#snippet result(row: Row, index: number)}
  {@const item = row.item}
  <button
    class="r"
    class:cur={index === at}
    class:off={item.kind === "action" && item.off}
    type="button"
    tabindex="-1"
    role="option"
    aria-selected={index === at}
    data-at={index}
    onclick={(e) => choose({ type: "row", row }, e.shiftKey)}
    onmousemove={(e) => hover(e, index)}
  >
    {#if item.kind === "file"}
      {@const parts = splitPath(item.path)}
      <span class="gl"><FileIcon name={parts.name} size={15} /></span>
      {@render split(parts.dir, parts.name, row.ranges, item.path)}
      {#if row.line !== undefined}<span class="meta">:{row.line}</span>{/if}
      <span class="sp"></span>
      <span class="meta">
        {#if item.changed}
          <DiffBar width={28} added={item.added} removed={item.removed} seen={item.seen} old={item.oldBase} />
          {#if item.status}{@render letter(item.status)}{/if}
        {:else}
          <span>unchanged</span>
        {/if}
      </span>
    {:else if item.kind === "symbol"}
      <span class="gl"><SymbolGlyph kind={item.symbolKind} /></span>
      {@render split(item.label.slice(0, item.lead), item.label.slice(item.lead), row.ranges, item.label)}
      <span class="sp"></span>
      <span class="meta">
        <span>{shortPath(item.info.id.path)}:{item.info.span.start.line + 1}</span>
        {#if item.change}{@render letter(SYMBOL_LETTERS[item.change])}{:else}<span class="ltr"></span>{/if}
      </span>
    {:else if item.kind === "changeset"}
      <span class="gl"><Icon name={item.icon} size={14} /></span>
      <span class="lab">{@render marked(item.number ? `${item.number} ${item.title}` : item.title, row.ranges)}</span>
      {#if item.onView}<span class="badge">on view</span>{/if}
      <span class="sp"></span>
      <span class="meta">
        <span>{item.source}</span>
        {#if item.finished}
          <span class="done"><Icon name="check" size={12} stroke={2.4} /></span>
        {/if}
        {#if item.seen !== undefined}
          <Meter width={48} segments={[{ value: item.seen, color: "var(--seen)" }]} />
          {#if !item.finished}<span class="pct">{Math.round(item.seen)}%</span>{/if}
        {:else}
          <span>—</span>
        {/if}
      </span>
    {:else if item.kind === "thread"}
      <span class="gl thread"><Icon name="comments" size={14} /></span>
      <span class="lab" class:resolved={item.resolved}>{@render marked(item.text, row.ranges)}</span>
      {#if item.moved}<span class="badge badge-moved">moved</span>{/if}
      {#if item.resolved}<span class="badge badge-ok">resolved</span>{/if}
      <span class="sp"></span>
      <span class="meta">{item.where === "changeset" ? item.where : shortPath(item.where)}</span>
    {:else}
      <span class="gl"><Icon name="zap" size={14} /></span>
      <span class="lab">{@render marked(item.label, row.ranges)}</span>
      <span class="sp"></span>
      <span class="meta">
        {#if item.off}
          {#if item.disabled}<span class="prose">{item.disabled}</span>{/if}
        {:else if item.binding}
          <span class="keys">{item.binding}</span>
        {/if}
      </span>
    {/if}
  </button>
{/snippet}

{#snippet entry(e: Entry, index: number)}
  {#if e.type === "row"}
    {@render result(e.row, index)}
  {:else if e.type === "more"}
    <button
      class="r more"
      class:cur={index === at}
      type="button"
      tabindex="-1"
      role="option"
      aria-selected={index === at}
      data-at={index}
      onclick={() => choose(e, false)}
      onmousemove={(ev) => hover(ev, index)}
    >
      <span class="gl"></span>
      <span class="lab">Show all {e.count} {TITLES[e.kind].toLowerCase()}</span>
      <span class="sp"></span>
      <span class="keys">↵</span>
    </button>
  {:else}
    <button
      class="r"
      class:cur={index === at}
      class:off={findOff !== ""}
      type="button"
      tabindex="-1"
      role="option"
      aria-selected={index === at}
      data-at={index}
      onclick={() => choose(e, false)}
      onmousemove={(ev) => hover(ev, index)}
    >
      <span class="gl"><Icon name="search" size={14} /></span>
      <span class="lab">Find “{e.query}” in files…</span>
      <span class="sp"></span>
      <span class="keys">{commands.binding(FIND_IN_FILES)?.label}</span>
    </button>
  {/if}
{/snippet}

<div class="surface palette" role="dialog" aria-label="Search everywhere" tabindex="-1" bind:this={root} {onkeydown}>
  <div class="tabs" role="tablist" aria-label="Search in">
    {#each TABS as tab (tab.id)}
      <button
        class="tab"
        type="button"
        role="tab"
        tabindex="-1"
        aria-selected={palette.tab === tab.id}
        onclick={() => {
          palette.tab = tab.id;
          input?.focus();
        }}
      >
        {tab.label}{#if built.counts}
          <span class="count">{built.counts[tab.id]}</span>{/if}
      </button>
    {/each}
    <span class="grow"></span>
    <button
      class="flat tog"
      type="button"
      tabindex="-1"
      aria-pressed={palette.unchanged}
      disabled={!TOGGLES.includes(palette.tab)}
      title={TOGGLES.includes(palette.tab)
        ? "Also search the files the changeset does not change"
        : "Applies to All, Files and Symbols"}
      onclick={() => {
        palette.setUnchanged(!palette.unchanged);
        input?.focus();
      }}
    >
      <span class="cb"
        >{#if palette.unchanged}<Icon name="check" size={9} stroke={3.2} />{/if}</span
      >Unchanged files
    </button>
  </div>
  <div class="query">
    <Icon name="search" size={14} stroke={1.7} />
    <input
      bind:this={input}
      bind:value={palette.query}
      type="text"
      spellcheck="false"
      autocomplete="off"
      aria-label="Search everywhere"
      placeholder="Search files, symbols, changesets, threads and actions"
    />
    {#if pinned}
      <span class="at-chip" title="Files open in a tab pinned to this commit"
        ><Icon name="gitCommit" size={11} stroke={1.8} />at {pinned}</span
      >
    {/if}
  </div>
  <div class="list" role="listbox" aria-label="Results" bind:this={list}>
    {#each built.groups as group, gi (gi)}
      {#if group.label}
        <div class="group">
          {group.label}{#if group.count !== undefined && palette.tab !== "all"}<span class="gn">{group.count}</span
            >{/if}{#if group.loading}<Spinner label={group.loading} /><span class="wait">{group.loading}</span>{/if}
        </div>
      {/if}
      {#each group.entries as e, ei (ei)}{@render entry(e, starts[gi] + ei)}{/each}
      {#if group.cut}<div class="cut">{group.cut} more, keep typing to narrow them</div>{/if}
    {/each}
    {#each failed as f (f.language)}
      <div class="note">
        <span class="msg" title={f.message}>{f.server} could not search: {f.message}</span>
        {#if f.actions.includes("restart")}
          <button class="ctl" type="button" tabindex="-1" onclick={() => void serverList.restart(f.language)}
            >Restart</button
          >
        {/if}
        {#if f.actions.includes("settings")}
          <button class="ctl" type="button" tabindex="-1" onclick={() => finish(onsettings)}>Settings</button>
        {/if}
      </div>
    {/each}
    {#each indexing as text (text)}<div class="note"><span class="msg">{text}</span></div>{/each}
    {#if !entries.length && !built.groups.some((g) => g.loading)}
      <div class="empty">
        {#if palette.query.trim()}
          <span class="big">No results for “{palette.query.trim()}”</span>
          {#if hidden}<span
              >{hidden} unchanged {hidden === 1 ? "file matches" : "files match"}, and are not searched.</span
            >{/if}
          <div class="acts">
            {#if hidden}
              <button class="ctl" type="button" tabindex="-1" onclick={() => palette.setUnchanged(true)}
                >Include unchanged files</button
              >
            {/if}
            <button
              class="ctl"
              type="button"
              tabindex="-1"
              disabled={!commands.isEnabled(FIND_IN_FILES)}
              title={findOff}
              onclick={() => choose({ type: "find", query: palette.query.trim() }, false)}
              >Find “{palette.query.trim()}” in files…
              <span class="keys">{commands.binding(FIND_IN_FILES)?.label}</span></button
            >
          </div>
        {:else}
          <span class="big">Nothing to list here yet.</span>
        {/if}
      </div>
    {/if}
  </div>
  <div class="foot">
    <span><b>↑↓</b> move</span>
    <span><b>↵</b> open</span>
    <span><b>⇧↵</b> show in All changes</span>
    <span><b>{NEXT_TAB}</b> next tab</span>
    <span><b>esc</b> close</span>
    <span class="grow"></span>
    {#if hint}
      <span>{hint}</span>
    {:else}
      <span class="where">
        {#if pinned}<Icon name="gitCommit" size={11} stroke={1.8} />Files open at {pinned}{:else}Files open on All
          changes{/if}
      </span>
    {/if}
  </div>
</div>

<style>
  .palette {
    position: fixed;
    z-index: 1000;
    top: calc(var(--h-header) + 6px);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    width: min(var(--w-palette), calc(100vw - 16px));
    max-height: min(var(--h-palette), calc(100vh - var(--h-header) - 40px));
    box-sizing: border-box;
    overflow: hidden;
    font-size: var(--fs);
    color: var(--text);
    background: var(--bg-editor);
  }
  .tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    height: var(--h-panel-tabs);
    padding: 0 8px 0 6px;
    border-bottom: 1px solid var(--line-soft);
    flex: none;
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 9px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-2);
    font: inherit;
    font-size: var(--fs-ctl);
  }
  .tab:hover,
  .tab[aria-selected="true"] {
    color: var(--text);
  }
  .tab[aria-selected="true"] {
    border-bottom-color: var(--accent);
  }
  .tog {
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 8px;
    border-radius: var(--radius);
    font-size: var(--fs-ctl);
    color: var(--text-2);
  }
  .tog:disabled {
    opacity: 0.5;
  }
  .tog .cb {
    width: 12px;
    height: 12px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--line);
    border-radius: 3px;
    background: var(--bg-control);
  }
  .tog[aria-pressed="true"] .cb {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg-editor);
  }
  .wait {
    font-weight: 400;
  }
  .cut {
    padding: 4px 18px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 18px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
  .note .msg {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count,
  .gn {
    color: var(--text-3);
    font: var(--fs-mono-s) var(--font-mono);
  }
  .query {
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--h-palette-field);
    padding: 0 12px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-3);
    flex: none;
  }
  .query input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text);
    font: 14px var(--font-ui);
  }
  .at-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 7px;
    border: 1px solid color-mix(in srgb, var(--accent) 38%, transparent);
    border-radius: 10px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    font: var(--fs-mono-s) var(--font-mono);
    white-space: nowrap;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 6px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px 3px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-3);
  }
  .group .gn {
    font-weight: 400;
  }
  .r {
    display: flex;
    align-items: center;
    gap: 9px;
    width: calc(100% - 12px);
    height: var(--h-palette-row);
    margin: 0 6px;
    padding: 0 12px;
    box-sizing: border-box;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    white-space: nowrap;
  }
  .r.cur {
    background: var(--select);
  }
  .r.off {
    color: var(--text-3);
  }
  .gl {
    flex: none;
    width: 16px;
    display: inline-flex;
    justify-content: center;
    color: var(--text-3);
  }
  .gl.thread {
    color: var(--note);
  }
  .lab {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .lab.resolved {
    color: var(--text-2);
  }
  .sp {
    flex: 1;
    min-width: 6px;
  }
  .meta {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font: var(--fs-mono-s) var(--font-mono);
  }
  .meta .ltr {
    width: 10px;
    text-align: center;
    font-size: 11px;
  }
  .meta .pct {
    width: 30px;
    text-align: right;
  }
  .meta .prose {
    font-family: var(--font-ui);
  }
  .done {
    display: inline-flex;
    color: var(--ok);
  }
  .keys {
    flex: none;
    color: var(--text-3);
    font: var(--fs-mono-s) var(--font-mono);
    letter-spacing: 0.02em;
  }
  .r.more {
    height: 24px;
    color: var(--accent);
    font-size: var(--fs-ctl);
  }
  mark {
    background: transparent;
    color: var(--accent);
    font-weight: 650;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 22px 16px 20px;
    color: var(--text-2);
    text-align: center;
  }
  .empty .big {
    color: var(--text);
  }
  .acts {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    justify-content: center;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 14px;
    height: var(--h-palette-foot);
    padding: 0 12px;
    border-top: 1px solid var(--line-soft);
    background: var(--bg-panel);
    font-size: 11.5px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    flex: none;
  }
  .foot b {
    font: 500 var(--fs-mono-s) var(--font-mono);
    color: var(--text-2);
  }
  .where {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--text-2);
  }
</style>

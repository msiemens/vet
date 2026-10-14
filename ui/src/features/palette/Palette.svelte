<script lang="ts">
  // Search Everywhere: one popup under the header for names (files, symbols, changesets, comment
  // threads, actions), with a tab for each kind. Arrow keys move, Tab or Ctrl+Tab changes the tab, Enter chooses,
  // Shift+Enter shows a changed file or symbol in All changes, Escape closes.
  import { tick, untrack } from "svelte";
  import { letterOfChange, letterOfStatus } from "../../lib/changeLetter";
  import { bindingOf, matches } from "../../lib/keymap";
  import { splitPath } from "../../lib/paths";
  import ChangeLetter from "../../lib/ui/ChangeLetter.svelte";
  import DiffBar from "../../lib/ui/DiffBar.svelte";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import FilterChip from "../../lib/ui/FilterChip.svelte";
  import GroupLabel from "../../lib/ui/GroupLabel.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import KeyHint from "../../lib/ui/KeyHint.svelte";
  import Kbd from "../../lib/ui/Kbd.svelte";
  import { ListNav, revealCurrent } from "../../lib/ui/listNav";
  import Marked from "../../lib/ui/Marked.svelte";
  import PanelTabs from "../../lib/ui/PanelTabs.svelte";
  import PopupShell from "../../lib/ui/PopupShell.svelte";
  import SeenMeter from "../../lib/ui/SeenMeter.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import SplitText from "../../lib/ui/SplitText.svelte";
  import TagChip from "../../lib/ui/TagChip.svelte";
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
  import { openFindInFiles } from "../search";
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

  /** How long the query rests before the language servers are asked. */
  const SERVER_PAUSE_MS = 120;
  /** The tabs the Unchanged files toggle applies to. */
  const TOGGLES: readonly PaletteTab[] = ["all", "file", "symbol"];
  const SYMBOL_TABS: readonly PaletteTab[] = ["all", "symbol"];
  const NEXT_TAB = bindingOf("Ctrl-Tab", isMac ? "Meta" : "Ctrl").label;

  let cursor = $state(0);
  let list = $state<HTMLElement>();
  let input = $state<HTMLInputElement>();

  const files = $derived(
    fileCandidates(
      (changeset.layout?.files ?? []).map((f) => {
        const completion = changeset.reviewed(f.path)?.completion;
        return {
          path: f.path,
          status: letterOfStatus(f.status),
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
              ? { kind: "progress", seen: reviewed.seen, oldBase: reviewed.oldBase }
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

  const nav = new ListNav<Entry>({
    items: () => entries,
    single: (entry) => choose(entry, false),
    highlight: { get: () => at, set: (i) => (cursor = i) },
    reveal: () => void revealCurrent(list),
    field: true,
  });

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
      finish(() => openFindInFiles({ query: palette.query.trim() || undefined }));
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
    if (e.key === "Enter" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
      take();
      choose(current, true);
      return;
    }
    if (nav.keydown(e)) take();
  }

  /** The last two parts of a path, which is all a row has room for. */
  function shortPath(path: string): string {
    return path.split("/").slice(-2).join("/");
  }
</script>

{#snippet result(row: Row, index: number)}
  {@const item = row.item}
  <button
    class="pick r"
    class:cur={index === at}
    class:off={item.kind === "action" && item.off}
    type="button"
    tabindex="-1"
    role="option"
    aria-selected={index === at}
    onclick={(e) => choose({ type: "row", row }, e.shiftKey)}
    onmousemove={(e) => nav.hover(e, index)}
  >
    {#if item.kind === "file"}
      {@const parts = splitPath(item.path)}
      <span class="gl"><FileIcon name={parts.name} size={15} /></span>
      <SplitText lead={parts.dir} last={parts.name} title={item.path} ranges={row.ranges} />
      {#if row.line !== undefined}<span class="meta">:{row.line}</span>{/if}
      <span class="sp"></span>
      <span class="meta">
        {#if item.changed}
          <DiffBar width={28} added={item.added} removed={item.removed} seen={item.seen} old={item.oldBase} />
          {#if item.status}<ChangeLetter letter={item.status} />{/if}
        {:else}
          <span>unchanged</span>
        {/if}
      </span>
    {:else if item.kind === "symbol"}
      <span class="gl"><SymbolGlyph kind={item.symbolKind} /></span>
      <SplitText
        lead={item.label.slice(0, item.lead)}
        last={item.label.slice(item.lead)}
        title={item.label}
        ranges={row.ranges}
      />
      <span class="sp"></span>
      <span class="meta">
        <span>{shortPath(item.info.id.path)}:{item.info.span.start.line + 1}</span>
        <ChangeLetter letter={item.change && letterOfChange(item.change)} />
      </span>
    {:else if item.kind === "changeset"}
      <span class="gl"><Icon name={item.icon} size={14} /></span>
      <span class="lab"
        ><Marked text={item.number ? `${item.number} ${item.title}` : item.title} ranges={row.ranges} /></span
      >
      {#if item.onView}<span class="badge">on view</span>{/if}
      <span class="sp"></span>
      <span class="meta">
        <span>{item.source}</span>
        {#if item.seen !== undefined}
          <SeenMeter width={48} seen={item.seen} old={item.oldBase} pct={!item.finished} done={item.finished} />
        {:else}
          <span>—</span>
        {/if}
      </span>
    {:else if item.kind === "thread"}
      <span class="gl thread"><Icon name="comments" size={14} /></span>
      <span class="lab" class:resolved={item.resolved}><Marked text={item.text} ranges={row.ranges} /></span>
      {#if item.moved}<span class="badge badge-moved">moved</span>{/if}
      {#if item.resolved}<span class="badge badge-ok">resolved</span>{/if}
      <span class="sp"></span>
      <span class="meta">{item.where === "changeset" ? item.where : shortPath(item.where)}</span>
    {:else}
      <span class="gl"><Icon name="zap" size={14} /></span>
      <span class="lab"><Marked text={item.label} ranges={row.ranges} /></span>
      <span class="sp"></span>
      <span class="meta">
        {#if item.off}
          {#if item.disabled}<span class="prose">{item.disabled}</span>{/if}
        {:else if item.binding}
          <Kbd keys={item.binding} />
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
      class="pick r more"
      class:cur={index === at}
      type="button"
      tabindex="-1"
      role="option"
      aria-selected={index === at}
      onclick={() => choose(e, false)}
      onmousemove={(ev) => nav.hover(ev, index)}
    >
      <span class="gl"></span>
      <span class="lab">Show all {e.count} {TITLES[e.kind].toLowerCase()}</span>
      <span class="sp"></span>
      <Kbd keys="↵" />
    </button>
  {:else}
    <button
      class="pick r"
      class:cur={index === at}
      class:off={findOff !== ""}
      type="button"
      tabindex="-1"
      role="option"
      aria-selected={index === at}
      onclick={() => choose(e, false)}
      onmousemove={(ev) => nav.hover(ev, index)}
    >
      <span class="gl"><Icon name="search" size={14} /></span>
      <span class="lab">Find “{e.query}” in files…</span>
      <span class="sp"></span>
      <Kbd keys={commands.binding(FIND_IN_FILES)?.label ?? ""} />
    </button>
  {/if}
{/snippet}

<PopupShell
  label="Search everywhere"
  placeholder="Search files, symbols, changesets, threads and actions"
  onclose={() => palette.close()}
  bind:value={palette.query}
  bind:input
  bind:body={list}
  {onkeydown}
>
  {#snippet head()}
    <div class="tabs">
      <PanelTabs
        label="Search in"
        focusable={false}
        tabs={TABS.map((t) => ({ id: t.id, label: t.label, count: built.counts?.[t.id] }))}
        value={palette.tab}
        onchange={(id) => {
          palette.tab = id;
          input?.focus();
        }}
      />
      <span class="grow"></span>
      <FilterChip
        focusable={false}
        on={palette.unchanged}
        disabled={!TOGGLES.includes(palette.tab)}
        title={TOGGLES.includes(palette.tab)
          ? "Also search the files the changeset does not change"
          : "Applies to All, Files and Symbols"}
        onclick={() => {
          palette.setUnchanged(!palette.unchanged);
          input?.focus();
        }}>Unchanged files</FilterChip
      >
    </div>
  {/snippet}
  {#snippet trailing()}
    {#if pinned}
      <TagChip mono title="Files open in a tab pinned to this commit"
        ><Icon name="commit" size={11} stroke={1.8} />at {pinned}</TagChip
      >
    {/if}
  {/snippet}
  <div role="listbox" aria-label="Results">
    {#each built.groups as group, gi (gi)}
      {#if group.label}
        <GroupLabel count={palette.tab !== "all" ? group.count : undefined}>
          {group.label}
          {#snippet trailing()}
            {#if group.loading}<Spinner label={group.loading} /><span class="wait">{group.loading}</span>{/if}
          {/snippet}
        </GroupLabel>
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
      {#if palette.query.trim()}
        <EmptyState
          title="No results for “{palette.query.trim()}”"
          text={hidden
            ? `${hidden} unchanged ${hidden === 1 ? "file matches" : "files match"}, and are not searched.`
            : undefined}
        >
          {#snippet actions()}
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
              <Kbd keys={commands.binding(FIND_IN_FILES)?.label ?? ""} /></button
            >
          {/snippet}
        </EmptyState>
      {:else}
        <EmptyState title="Nothing to list here yet." />
      {/if}
    {/if}
  </div>
  {#snippet foot()}
    <KeyHint keys="↑↓">move</KeyHint>
    <KeyHint keys="↵">open</KeyHint>
    <KeyHint keys="⇧↵">show in All changes</KeyHint>
    <KeyHint keys={NEXT_TAB}>next tab</KeyHint>
    <KeyHint keys="esc">close</KeyHint>
    <span class="grow"></span>
    {#if hint}
      <span>{hint}</span>
    {:else}
      <span class="where">
        {#if pinned}<Icon name="commit" size={11} stroke={1.8} />Files open at {pinned}{:else}Files open on All changes{/if}
      </span>
    {/if}
  {/snippet}
</PopupShell>

<style>
  .tabs {
    display: flex;
    align-items: center;
    padding: 0 8px 0 0;
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
  .r.off {
    color: var(--text-3);
  }
  .gl {
    flex: none;
    width: var(--s-icon);
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
  .meta .prose {
    font-family: var(--font-ui);
  }
  .r.more {
    height: 24px;
    color: var(--accent);
    font-size: var(--fs-ctl);
  }
  .where {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--text-2);
  }
</style>

<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { SearchLine } from "../../generated/SearchLine";
  import type { SearchSide } from "../../generated/SearchSide";
  import type { View } from "../../generated/View";
  import { letterOfStatus } from "../../lib/changeLetter";
  import { Debounced } from "../../lib/debounced";
  import { errorText } from "../../lib/errorText";
  import { toggleIn } from "../../lib/foldKeys";
  import { count, noun, plural } from "../../lib/format";
  import { segments, trimmed } from "../../lib/foundText";
  import { basename, dirname } from "../../lib/paths";
  import { NO_HIGHLIGHTS } from "../../lib/highlights";
  import { SEARCH_OPTION_TOGGLES } from "../../lib/search/optionToggles";
  import ChangeLetter from "../../lib/ui/ChangeLetter.svelte";
  import { changeGate } from "../../lib/ui/changeGate";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import FilterChip from "../../lib/ui/FilterChip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import KeyHint from "../../lib/ui/KeyHint.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import Menu from "../../lib/ui/Menu.svelte";
  import type { MenuItem } from "../../lib/ui/menuItem";
  import OpenInFindButton from "../../lib/ui/OpenInFindButton.svelte";
  import SeenMeter from "../../lib/ui/SeenMeter.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import PopupShell from "../../lib/ui/PopupShell.svelte";
  import SearchOptionToggles from "../../lib/ui/SearchOptionToggles.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import TreeRow from "../../lib/ui/TreeRow.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import type { Backend, SearchQuery, SearchResults } from "../../state/backend";
  import { commands } from "../../state/commands";
  import type { KeptSearch } from "../../state/findSets.svelte";
  import { isMac } from "../../state/host";
  import { findInFiles } from "./FindInFiles.svelte";
  import { LatestRun } from "./latestRun";
  import ResultPreview from "./ResultPreview.svelte";
  import {
    type OpenedResult,
    type ResultRow,
    type ScopeChoice,
    resolveScope,
    resultId,
    resultRows,
    rowKey,
    scopeChoices,
    selectedRow,
    viewScopeId,
  } from "./searchModel";

  let {
    backend,
    view,
    files,
    range,
    revision,
    onopen,
    onkeep,
  }: {
    backend: Backend;
    /** The view the review shows. */
    view: View;
    /** How many files the changeset changes. */
    files: number;
    /** The versions a search can compare, if the review has several. */
    range?: { from: number; to: number };
    /** The changeset's revision: a new one runs the search again. */
    revision: number;
    /** Shows a result; `ownTab` is asked for by the viewer. The dialog is closed already. */
    onopen: (result: OpenedResult) => void;
    /** Keeps the results in the Find tool. The dialog is closed already. */
    onkeep: (kept: KeptSearch) => void;
  } = $props();

  /** The most matches a search lists. */
  const LIMIT = 1000;
  /** The keys that move the highlight, which a focused button leaves to the list. */
  const MOVES = ["ArrowDown", "ArrowUp", "PageDown", "PageUp"];
  const find = findInFiles;

  let commits = $state<{ shortId: string; subject: string; id: string }[]>([]);
  let results = $state.raw<SearchResults>();
  let error = $state<string>();
  let pending = $state(false);
  /** The files whose results are folded away. */
  const collapsed = new SvelteSet<string>();
  let scopeOpen = $state(false);
  let input = $state<HTMLInputElement>();
  let list = $state<VirtualRows>();
  const latest = new LatestRun<SearchResults>();

  const choices = $derived(
    scopeChoices({ view, files, range, commits: commits.map(({ shortId, subject }) => ({ shortId, subject })) }),
  );
  const choice = $derived<ScopeChoice>(resolveScope(choices, find.scope ?? viewScopeId(view)));
  const repository = $derived(choice.scope.type === "repository");
  const rows = $derived(resultRows(results?.files ?? [], collapsed));
  const at = $derived(selectedRow(rows, find.selected));
  const current = $derived(at >= 0 ? rows[at] : undefined);

  onMount(() => {
    void backend.commits().then(
      ({ commits: units }) =>
        (commits = units.map((u) => ({ shortId: u.commit.shortId, subject: u.commit.subject, id: u.commit.id }))),
      () => undefined,
    );
  });

  // The cursor goes to the query each time the dialog is asked for.
  const focusRequested = changeGate();
  $effect(() => {
    if (focusRequested(find.focusRequests)) {
      void tick().then(() => {
        input?.focus();
        input?.select();
      });
    }
  });

  const asked = (): SearchQuery => ({
    scope: choice.scope,
    pattern: { text: find.text, matchCase: find.matchCase, wholeWord: find.wholeWord, regex: find.regex },
    mask: find.mask,
    side: find.side,
    onlyChanges: find.onlyChanges,
    notSeen: find.notSeen,
    limit: LIMIT,
  });

  // The search runs as the query, the options or the changeset change; an answer to an older one is dropped.
  const reasked = changeGate();
  $effect(() => {
    const request = asked();
    // The gate reads the revision too, so a new changeset runs the search again.
    if (!reasked(revision, request)) return;
    if (!request.pattern.text) {
      latest.cancel();
      results = undefined;
      error = undefined;
      pending = false;
      return;
    }
    pending = true;
    const pause = new Debounced();
    pause.schedule(120, () => {
      void latest
        .run((signal) => backend.search(request, signal))
        .then((outcome) => {
          if (!outcome) return;
          pending = false;
          if (outcome.type === "done") {
            results = outcome.value;
            error = undefined;
          } else {
            results = undefined;
            error = errorText(outcome.error);
          }
        });
    });
    return () => pause.cancel();
  });

  onDestroy(() => latest.cancel());

  const maskError = $derived(error?.startsWith('invalid pattern "') === true);

  const summary = $derived.by(() => {
    if (!find.text) return "";
    if (error !== undefined) return "";
    if (!results || pending) return "Searching…";
    const { hits, files: found, truncated } = results;
    if (!hits) return "0 matches";
    const text = `${count(hits)}${truncated ? "+" : ""} ${noun(hits, "match", "matches")} in ${plural(found.length, "file")}`;
    return truncated ? `${text}, narrow the query` : text;
  });

  const sides: { id: SearchSide; label: string }[] = [
    { id: "both", label: "Both" },
    { id: "target", label: "Target" },
    { id: "base", label: "Base" },
  ];
  const sideOptions = $derived(
    sides.map((s) => ({
      ...s,
      disabled: repository,
      title: repository ? "A search of the whole repository reads the target's text only" : undefined,
    })),
  );

  const scopeItems = $derived.by((): MenuItem[] => {
    const groups: [ScopeChoice["group"], string][] = [
      ["view", "The view on show"],
      ["commit", "A commit of this changeset"],
      ["repository", "Beyond the changeset"],
    ];
    return groups.flatMap(([group, title], i) => {
      const inGroup = choices.filter((c) => c.group === group);
      if (!inGroup.length) return [];
      return [
        ...(i ? [{ id: `-${group}`, label: "", separator: true } satisfies MenuItem] : []),
        { id: `#${group}`, label: title, enabled: false },
        ...inGroup.map((c): MenuItem => ({
          id: c.id,
          label: c.label,
          detail: c.detail,
          checked: c.id === choice.id,
          radio: true,
        })),
      ];
    });
  });

  function chooseScope(id: string) {
    find.scope = id;
    scopeOpen = false;
    input?.focus();
  }

  function select(index: number) {
    const row = rows.at(index);
    if (row?.kind !== "line") return;
    find.selected = resultId(row.file.path, row.lineIndex);
    // The first result of a file shows its header too.
    list?.reveal(index);
    if (rows[index - 1]?.kind === "file") list?.reveal(index - 1);
  }

  /** The indices of the rows that are results; the file headers are no stops. */
  const lines = $derived(rows.flatMap((r, i) => (r.kind === "line" ? [i] : [])));
  const nav = new ListNav<number>({
    items: () => lines,
    single: (i) => open(rows[i], false),
    highlight: {
      get: () => lines.indexOf(at),
      set: (n) => {
        if (n >= 0) select(lines[n]);
      },
    },
    field: true,
  });

  const toggleFile = (path: string) => toggleIn(collapsed, path);

  function open(row: ResultRow | undefined, ownTab: boolean) {
    if (row?.kind !== "line") return;
    const { file, line } = row;
    const first = line.hits[0]?.from ?? 0;
    const index =
      choice.scope.type === "view" && choice.scope.view.type === "commit" ? choice.scope.view.index : undefined;
    find.close();
    onopen({
      path: file.path,
      hunks: file.hunks,
      kind: line.kind,
      line: line.line,
      col: first,
      width: (line.hits[0]?.to ?? first) - first,
      commit: index === undefined ? undefined : commits[index]?.id,
      scope: choice.scope,
      ownTab,
      options: {
        text: find.text,
        matchCase: find.matchCase,
        wholeWord: find.wholeWord,
        regex: find.regex,
        side: repository ? "target" : find.side,
        onlyChanges: find.onlyChanges,
      },
    });
  }

  /** What the viewer can keep: the results of the query as it stands, once they are in. */
  const keepable = $derived(!!results && !pending && error === undefined && !!find.text && results.files.length > 0);

  function keep() {
    if (!results || !keepable) return;
    const index =
      choice.scope.type === "view" && choice.scope.view.type === "commit" ? choice.scope.view.index : undefined;
    const unit = index === undefined ? undefined : commits[index];
    const kept: KeptSearch = {
      results,
      spec: {
        type: "search",
        request: asked(),
        scope: { label: choice.label, commit: unit && { id: unit.id, shortId: unit.shortId } },
        options: {
          text: find.text,
          matchCase: find.matchCase,
          wholeWord: find.wholeWord,
          regex: find.regex,
          side: repository ? "target" : find.side,
          onlyChanges: find.onlyChanges,
        },
      },
    };
    find.close();
    onkeep(kept);
  }

  /** Escape with the scope menu open closes it alone, before the popup reads the key. */
  function escapeKey(e: KeyboardEvent) {
    if (e.key !== "Escape" || !scopeOpen) return;
    e.preventDefault();
    e.stopPropagation();
    scopeOpen = false;
    input?.focus();
  }

  function keydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    if (e.altKey && !e.ctrlKey && e.metaKey === isMac && !e.shiftKey) {
      // Option types a letter, so the toggles are read by the key's place.
      const toggle = SEARCH_OPTION_TOGGLES.find((t) => t.code === e.code);
      if (toggle) {
        e.preventDefault();
        find[toggle.key] = !find[toggle.key];
        return;
      }
    }
    if (e.key === "Enter" && !e.altKey && !e.shiftKey && (isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey)) {
      e.preventDefault();
      keep();
      return;
    }
    if (target.closest(".mask")) return;
    const onControl = target.closest("button, [role=menuitem], [role=menuitemradio]") !== null;
    if (e.key === "Enter" && e.altKey && !e.ctrlKey && !e.metaKey && !onControl) {
      e.preventDefault();
      open(current, true);
      return;
    }
    if (onControl && !MOVES.includes(e.key)) return;
    if (target.closest("[role=menu]")) return;
    if (nav.keydown(e)) e.preventDefault();
  }

  /** The 1-based number of a result's line. */
  const numberOf = (row: ResultRow) => (row.kind === "line" ? row.line.line + 1 : 0);

  /** Shown where results would be. */
  const hint = $derived.by(() => {
    if (error !== undefined) return { text: error, bad: true };
    if (!find.text) {
      const where = repository ? "the repository" : "the changed files";
      return { text: `Type to search ${where}. Results group by file and open at their line.`, bad: false };
    }
    if (pending && !results) return { text: "Searching…", bad: false };
    if (results && !results.files.length)
      return { text: `No matches in ${plural(results.searched, "file")}.`, bad: false };
    return undefined;
  });

  function lineParts(line: SearchLine) {
    const { text, hits } = trimmed(line);
    return segments(text, 0, NO_HIGHLIGHTS, hits);
  }
</script>

{#snippet head()}
  <div class="top">
    <b class="title">Find in Files</b>
    <span class="grow"></span>
    <span class="in">In</span>
    <Popover bind:open={scopeOpen} align="right">
      {#snippet trigger(toggle)}
        <button
          class="scope"
          type="button"
          title="Where to search"
          aria-haspopup="menu"
          aria-expanded={scopeOpen}
          onclick={toggle}
        >
          <span class="label">{choice.label}</span>
          <span class="detail">{choice.detail}</span>
          <Icon name="open" size={10} />
        </button>
      {/snippet}
      <Menu items={scopeItems} onselect={chooseScope} />
    </Popover>
    <input
      class="field mask"
      class:bad={maskError}
      type="text"
      value={find.mask}
      placeholder="File mask  *.rs, !tests/**"
      aria-label="File mask"
      title="Globs separated by commas; ! leaves a file out"
      spellcheck="false"
      autocomplete="off"
      oninput={(e) => (find.mask = e.currentTarget.value)}
    />
  </div>
{/snippet}

{#snippet trailing()}
  <SearchOptionToggles
    values={find}
    title={(t) =>
      t.key === "regex"
        ? `${commands.tooltip(t.id, t.name)}. Rust syntax here; the find bar uses JavaScript's`
        : commands.tooltip(t.id, t.name)}
    onchange={(key) => (find[key] = !find[key])}
  />
{/snippet}

<svelte:window onkeydowncapture={escapeKey} />

<PopupShell
  wide
  scroll={false}
  label="Search text"
  placeholder="Search text"
  invalid={error !== undefined && !maskError}
  onclose={() => find.close()}
  onkeydown={keydown}
  bind:value={find.text}
  bind:input
  {head}
  {trailing}
>
  <div class="box">
    <div class="filters">
      <span class="summary">{summary}</span>
      <span class="grow"></span>
      <Segmented
        small
        label="Side"
        value={repository ? "target" : find.side}
        onchange={(s: SearchSide) => void (find.side = s)}
        options={sideOptions}
      />
      <FilterChip
        on={find.onlyChanges}
        title="Only lines the change added or removed"
        onclick={() => (find.onlyChanges = !find.onlyChanges)}>Only changes</FilterChip
      >
      <FilterChip
        on={find.notSeen}
        title="Only changed lines in hunks you have not marked seen"
        onclick={() => (find.notSeen = !find.notSeen)}>Not seen</FilterChip
      >
      <OpenInFindButton
        keys="{isMac ? '⌘' : 'Ctrl+'}↵"
        title="Keep these results in the Find window"
        disabled={!keepable}
        onclick={keep}
      />
    </div>
    <div class="results">
      {#if hint}
        <EmptyState text={hint.text} error={hint.bad} />
      {:else}
        <VirtualRows bind:this={list} count={rows.length} key={(i) => rowKey(rows[i])}>
          {#snippet row(i)}
            {@const r = rows[i]}
            {#if r.kind === "file"}
              {@const dir = dirname(r.file.path)}
              <TreeRow
                open={!r.collapsed}
                tone="strong"
                count={r.file.hits}
                letters={false}
                title={r.file.path}
                tabindex={-1}
                onclick={() => toggleFile(r.file.path)}
              >
                {#snippet glyph()}<FileIcon name={basename(r.file.path)} />{/snippet}
                <span class="name">{basename(r.file.path)}</span>
                {#if dir}<span class="dir">{dir}/</span>{/if}
                {#snippet trailing()}
                  {#if r.file.completion && r.file.status}
                    <SeenMeter
                      width={30}
                      title="{Math.round(r.file.completion.seenPercent)}% seen"
                      seen={r.file.completion.seenPercent}
                      old={r.file.completion.oldBasePercent}
                    />
                    <ChangeLetter letter={letterOfStatus(r.file.status)} title={r.file.status} />
                  {:else}
                    <span class="unchanged">unchanged</span>
                  {/if}
                {/snippet}
              </TreeRow>
            {:else}
              <!-- The row is chosen by the keys of the dialog; a click only chooses it too. -->
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div
                class="pick review-strip line {r.line.kind}"
                class:cur={i === at}
                data-state={r.line.hunk === undefined ? undefined : r.file.states[r.line.hunk]}
                onclick={() => select(i)}
                ondblclick={(e) => open(r, e.altKey)}
              >
                <span class="no">{numberOf(r)}</span>
                <span class="sign">{r.line.kind === "added" ? "+" : r.line.kind === "removed" ? "−" : ""}</span>
                <span class="text"
                  >{#each lineParts(r.line) as p, j (j)}{#if p.find}<mark>{p.text}</mark
                      >{:else}{p.text}{/if}{/each}</span
                >
              </div>
            {/if}
          {/snippet}
        </VirtualRows>
      {/if}
    </div>
    {#if current?.kind === "line"}
      <ResultPreview
        {backend}
        file={current.file}
        line={current.line}
        view={choice.scope.type === "view" ? choice.scope.view : { type: "changeset" }}
        {revision}
        {repository}
      />
    {/if}
  </div>
  {#snippet foot()}
    <KeyHint keys="↑↓">move</KeyHint>
    <KeyHint keys="↵">open</KeyHint>
    <KeyHint keys="{isMac ? '⌥' : 'Alt+'}↵">its own tab</KeyHint>
    <KeyHint keys="esc">close</KeyHint>
  {/snippet}
</PopupShell>

<style>
  .in {
    color: var(--text-3);
    font-size: var(--fs-xs);
    letter-spacing: 0.05em;
  }
  .scope {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 300px;
    height: var(--h-ctl-m);
    padding: 0 8px 0 9px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-control);
    font-size: var(--fs-mono);
    text-align: left;
  }
  .scope:hover {
    background: var(--hover);
  }
  .scope .label {
    font-weight: 600;
    white-space: nowrap;
  }
  .scope .detail {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-3);
  }
  .mask {
    flex: none;
    width: 190px;
    height: var(--h-ctl-m);
  }
  .mask.bad {
    color: var(--del-text);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--h-panel-tabs);
    padding: 0 12px;
  }
  .title {
    font-size: var(--fs-ctl);
    font-weight: 600;
  }
  .box {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line-soft);
  }
  .summary {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
    min-height: 16px;
  }
  .grow {
    flex: 1;
  }
  .results {
    flex: 1;
    min-height: 90px;
    display: flex;
    flex-direction: column;
  }
  .results > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .name {
    font-weight: 600;
  }
  .dir {
    margin-left: 6px;
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .unchanged {
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-row);
    padding: 0 8px 0 18px;
    font: var(--fs-mono) var(--font-mono);
    font-variant-ligatures: none;
  }
  .line.added {
    background: color-mix(in srgb, var(--add-bg) 60%, transparent);
  }
  .line.removed {
    background: color-mix(in srgb, var(--del-bg) 60%, transparent);
  }
  .line:hover {
    box-shadow: inset 0 0 0 100px var(--hover);
  }
  .no {
    width: 28px;
    flex: none;
    text-align: right;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .sign {
    width: 9px;
    flex: none;
    color: var(--text-3);
  }
  .added .sign {
    color: var(--add-text);
  }
  .removed .sign {
    color: var(--del-text);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
  }
  mark {
    background: var(--find);
    color: inherit;
    border-radius: 2px;
  }
</style>

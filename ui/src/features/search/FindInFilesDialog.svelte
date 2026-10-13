<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { SearchLine } from "../../generated/SearchLine";
  import type { SearchSide } from "../../generated/SearchSide";
  import type { View } from "../../generated/View";
  import { letterOfStatus } from "../../lib/changeLetter";
  import { errorText } from "../../lib/errorText";
  import { segments, trimmed } from "../../lib/foundText";
  import { basename, dirname } from "../../lib/paths";
  import { NO_HIGHLIGHTS } from "../../lib/highlights";
  import ChangeLetter from "../../lib/ui/ChangeLetter.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import KeyHint from "../../lib/ui/KeyHint.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import type { MenuItem } from "../../lib/ui/menuItem";
  import SeenMeter from "../../lib/ui/SeenMeter.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import PopupShell from "../../lib/ui/PopupShell.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
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
    stepLine,
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
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the request a dependency
    find.focusRequests;
    void tick().then(() => {
      input?.focus();
      input?.select();
    });
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
  $effect(() => {
    const request = asked();
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the revision a dependency
    revision;
    if (!request.pattern.text) {
      latest.cancel();
      results = undefined;
      error = undefined;
      pending = false;
      return;
    }
    pending = true;
    const timer = setTimeout(() => {
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
    }, 120);
    return () => clearTimeout(timer);
  });

  onDestroy(() => latest.cancel());

  const maskError = $derived(error?.startsWith('invalid pattern "') === true);

  const summary = $derived.by(() => {
    if (!find.text) return "";
    if (error !== undefined) return "";
    if (!results || pending) return "Searching…";
    const { hits, files: found, truncated } = results;
    if (!hits) return "0 matches";
    const count = `${hits.toLocaleString()}${truncated ? "+" : ""} ${hits === 1 ? "match" : "matches"} in ${found.length} ${found.length === 1 ? "file" : "files"}`;
    return truncated ? `${count}, narrow the query` : count;
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

  const toggles = [
    { key: "matchCase", label: "Aa", name: "Match case", code: "KeyC", id: "find.matchCase" },
    { key: "wholeWord", label: "ab", name: "Whole word", code: "KeyW", id: "find.wholeWord" },
    { key: "regex", label: ".*", name: "Regular expression", code: "KeyR", id: "find.regex" },
  ] as const;

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
    list?.reveal(index);
  }

  function move(delta: 1 | -1) {
    const to = stepLine(rows, at, delta);
    if (to === undefined) return;
    // Up from the first result still shows its file's header.
    select(to);
    if (delta < 0 && rows[to - 1]?.kind === "file") list?.reveal(to - 1);
  }

  function toggleFile(path: string) {
    if (!collapsed.delete(path)) collapsed.add(path);
  }

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
      const toggle = toggles.find((t) => t.code === e.code);
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
    if (target.closest("button, [role=menuitem], [role=menuitemradio]") && e.key !== "ArrowDown" && e.key !== "ArrowUp")
      return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (target.closest("[role=menu]")) return;
      e.preventDefault();
      move(e.key === "ArrowDown" ? 1 : -1);
    } else if (e.key === "Enter" && current) {
      e.preventDefault();
      open(current, e.altKey);
    }
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
      return { text: `No matches in ${results.searched} ${results.searched === 1 ? "file" : "files"}.`, bad: false };
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
  <span class="toggles">
    {#each toggles as t (t.key)}
      <button
        class="flat toggle"
        class:word={t.key === "wholeWord"}
        type="button"
        aria-pressed={find[t.key]}
        title={t.key === "regex"
          ? `${commands.tooltip(t.id, t.name)}. Rust syntax here; the find bar uses JavaScript's`
          : commands.tooltip(t.id, t.name)}
        onclick={() => (find[t.key] = !find[t.key])}>{t.label}</button
      >
    {/each}
  </span>
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
      <button
        class="ctl chip"
        type="button"
        aria-pressed={find.onlyChanges}
        title="Only lines the change added or removed"
        onclick={() => (find.onlyChanges = !find.onlyChanges)}>Only changes</button
      >
      <button
        class="ctl chip"
        type="button"
        aria-pressed={find.notSeen}
        title="Only changed lines in hunks you have not marked seen"
        onclick={() => (find.notSeen = !find.notSeen)}>Not seen</button
      >
      <button
        class="ctl keep"
        type="button"
        title="Keep these results in the Find window"
        disabled={!keepable}
        onclick={keep}>Open in Find window <kbd>{isMac ? "⌘" : "Ctrl+"}↵</kbd></button
      >
    </div>
    <div class="results">
      {#if hint}
        <p class="hint" class:bad={hint.bad}>{hint.text}</p>
      {:else}
        <VirtualRows bind:this={list} count={rows.length} key={(i) => rowKey(rows[i])}>
          {#snippet row(i)}
            {@const r = rows[i]}
            {#if r.kind === "file"}
              {@const dir = dirname(r.file.path)}
              <button
                class="flat head"
                type="button"
                tabindex="-1"
                title={r.file.path}
                onclick={() => toggleFile(r.file.path)}
              >
                <span class="chev"><Icon name={r.collapsed ? "closed" : "open"} size={10} /></span>
                <FileIcon name={basename(r.file.path)} />
                <span class="name">{basename(r.file.path)}</span>
                {#if dir}<span class="dir">{dir}/</span>{/if}
                <span class="grow"></span>
                <span class="count">{r.file.hits}</span>
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
              </button>
            {:else}
              <!-- The row is chosen by the keys of the dialog; a click only chooses it too. -->
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div
                class="line {r.line.kind}"
                class:sel={i === at}
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
    font-size: 11px;
    letter-spacing: 0.05em;
  }
  .scope {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 300px;
    height: 24px;
    padding: 0 8px 0 9px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-control);
    font-size: 12px;
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
    height: 24px;
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
  .toggles {
    display: inline-flex;
    gap: 1px;
  }
  .toggle {
    width: 24px;
    height: 22px;
    border-radius: 4px;
    color: var(--text-2);
    font: 600 11px var(--font-mono);
  }
  .toggle.word {
    text-decoration: underline;
    text-underline-offset: 2px;
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
    font-size: 11.5px;
    min-height: 16px;
  }
  .grow {
    flex: 1;
  }
  .chip {
    height: 22px;
    padding: 0 9px;
    border-radius: 11px;
    font-size: 11.5px;
  }
  .keep {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 9px;
    font-size: 11.5px;
  }
  .keep kbd {
    color: var(--text-3);
    font: 11px var(--font-mono);
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
  .hint {
    margin: 0;
    padding: 14px 12px;
    color: var(--text-3);
    font-size: 12px;
    flex: none;
  }
  .hint.bad {
    color: var(--del-text);
  }
  .head {
    width: 100%;
    height: var(--h-row);
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 0 6px;
    font-size: 12.5px;
    text-align: left;
  }
  .chev {
    display: inline-flex;
    width: 12px;
    color: var(--text-2);
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
  }
  .dir {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-3);
    font-size: 11.5px;
  }
  .count {
    min-width: 18px;
    height: 15px;
    box-sizing: border-box;
    padding: 0 5px;
    border-radius: 8px;
    background: var(--track);
    color: var(--text-2);
    font: 600 10.5px/15px var(--font-ui);
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .unchanged {
    color: var(--text-3);
    font-size: 11px;
  }
  .line {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-row);
    padding: 0 8px 0 18px;
    font: 12px var(--font-mono);
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
  .line.sel {
    background: var(--select);
  }
  .line[data-state="seen"]::before,
  .line[data-state="seenAgainstOldBase"]::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 3px;
    background: var(--seen);
  }
  .line[data-state="seenAgainstOldBase"]::before {
    background: var(--old-base-stripes);
  }
  .no {
    width: 28px;
    flex: none;
    text-align: right;
    color: var(--text-3);
    font-size: 11px;
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

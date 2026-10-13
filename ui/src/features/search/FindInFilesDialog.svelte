<script lang="ts">
  import { onMount, tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { SearchFile } from "../../generated/SearchFile";
  import type { SearchLine } from "../../generated/SearchLine";
  import type { SearchSide } from "../../generated/SearchSide";
  import type { View } from "../../generated/View";
  import { errorText } from "../../lib/errorText";
  import { basename, dirname } from "../../lib/paths";
  import { NO_HIGHLIGHTS } from "../../lib/highlights";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import type { MenuItem } from "../../lib/ui/menuItem";
  import Meter from "../../lib/ui/Meter.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import type { Backend, SearchResults } from "../../state/backend";
  import { commands } from "../../state/commands";
  import { isMac } from "../../state/host";
  import { findInFiles } from "./FindInFiles.svelte";
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
    segments,
    selectedRow,
    stepLine,
    trimmed,
    viewScopeId,
  } from "./searchModel";

  let {
    backend,
    view,
    files,
    range,
    revision,
    onopen,
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
  } = $props();

  /** The most matches a search lists. */
  const LIMIT = 1000;
  const STATUS_LETTER = { modified: "M", added: "A", deleted: "D", renamed: "R" } as const;
  const STATUS_COLOR = {
    M: "var(--old-base)",
    A: "var(--add-text)",
    D: "var(--del-text)",
    R: "var(--text-2)",
  } as const;
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
  let sequence = 0;
  // Escape closes the scope menu first and the dialog only when no menu was open.
  let menuWasOpen = false;

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

  // The search runs as the query, the options or the changeset change; an answer to an older one is dropped.
  $effect(() => {
    const request = {
      scope: choice.scope,
      pattern: { text: find.text, matchCase: find.matchCase, wholeWord: find.wholeWord, regex: find.regex },
      mask: find.mask,
      side: find.side,
      onlyChanges: find.onlyChanges,
      notSeen: find.notSeen,
      limit: LIMIT,
    };
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the revision a dependency
    revision;
    const n = ++sequence;
    if (!request.pattern.text) {
      results = undefined;
      error = undefined;
      pending = false;
      return;
    }
    pending = true;
    const timer = setTimeout(() => {
      backend.search(request).then(
        (r) => {
          if (n !== sequence) return;
          results = r;
          error = undefined;
          pending = false;
        },
        (e: unknown) => {
          if (n !== sequence) return;
          results = undefined;
          error = errorText(e);
          pending = false;
        },
      );
    }, 120);
    return () => clearTimeout(timer);
  });

  const maskError = $derived(error?.startsWith('invalid pattern "') === true);

  const summary = $derived.by(() => {
    if (!find.text) return "";
    if (error !== undefined) return "";
    if (!results) return pending ? "Searching…" : "";
    const { hits, files: found, truncated } = results;
    if (!hits) return "0 matches";
    const count = `${hits.toLocaleString()}${truncated ? "+" : ""} ${hits === 1 ? "match" : "matches"} in ${found.length} ${found.length === 1 ? "file" : "files"}`;
    return truncated ? `${count}. Only the first ${LIMIT.toLocaleString()} are listed; narrow the query` : count;
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
      ["view", "The changeset"],
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

  function keydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    if (e.key === "Escape") {
      menuWasOpen = scopeOpen;
      return;
    }
    if (e.altKey && !e.ctrlKey && e.metaKey === isMac && !e.shiftKey) {
      // Option types a letter, so the toggles are read by the key's place.
      const toggle = toggles.find((t) => t.code === e.code);
      if (toggle) {
        e.preventDefault();
        find[toggle.key] = !find[toggle.key];
        return;
      }
    }
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

  function close() {
    if (menuWasOpen) {
      menuWasOpen = false;
      scopeOpen = false;
      return;
    }
    find.close();
  }

  /** The 1-based number of a result's line. */
  const numberOf = (row: ResultRow) => (row.kind === "line" ? row.line.line + 1 : 0);

  const letterOf = (file: SearchFile) => (file.status ? STATUS_LETTER[file.status] : undefined);

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

{#snippet bar()}
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
{/snippet}

<Dialog title="Find in Files" width="840px" height="640px" padded={false} onclose={close} {bar}>
  <!-- The keys of the controls inside are read here. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="box" onkeydown={keydown}>
    <div class="query">
      <span class="glyph"><Icon name="search" size={15} /></span>
      <input
        bind:this={input}
        class:bad={error !== undefined && !maskError}
        type="text"
        value={find.text}
        placeholder="Search text"
        aria-label="Search text"
        spellcheck="false"
        autocomplete="off"
        oninput={(e) => (find.text = e.currentTarget.value)}
      />
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
    </div>
    <div class="filters">
      <span class="summary">{summary}</span>
      {#if pending && results}<Spinner label="Searching" />{/if}
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
    </div>
    <div class="results">
      {#if hint}
        <p class="hint" class:bad={hint.bad}>{hint.text}</p>
      {:else}
        <VirtualRows bind:this={list} count={rows.length} key={(i) => rowKey(rows[i])}>
          {#snippet row(i)}
            {@const r = rows[i]}
            {#if r.kind === "file"}
              {@const letter = letterOf(r.file)}
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
                {#if r.file.completion && letter}
                  <Meter
                    width={30}
                    title="{Math.round(r.file.completion.seenPercent)}% seen"
                    segments={[
                      { value: r.file.completion.seenPercent, color: "var(--seen)" },
                      { value: r.file.completion.oldBasePercent, color: "var(--old-base-stripes)" },
                    ]}
                  />
                  <span class="mono letter" style:color={STATUS_COLOR[letter]} title={r.file.status}>{letter}</span>
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
        status={current.file.status}
        letter={letterOf(current.file)}
      />
    {/if}
    <div class="keys">
      <span><kbd>↑</kbd> <kbd>↓</kbd> choose</span>
      <span><kbd>Enter</kbd> open</span>
      <span><kbd>{isMac ? "⌥" : "Alt+"}Enter</kbd> its own tab</span>
      <span><kbd>Esc</kbd> close</span>
    </div>
  </div>
</Dialog>

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
  .mask.bad,
  .query input.bad {
    color: var(--del-text);
  }
  .box {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    border-radius: inherit;
  }
  .query {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 42px;
    flex: none;
    padding: 0 12px;
    border-bottom: 1px solid var(--line-soft);
  }
  .glyph {
    display: inline-flex;
    color: var(--text-3);
  }
  .query input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font: 15px var(--font-ui);
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
  .letter {
    width: 12px;
    text-align: center;
    font-size: 11.5px;
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
    background: var(--toggle);
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
  .keys {
    display: flex;
    gap: 16px;
    flex: none;
    padding: 6px 12px;
    border-top: 1px solid var(--line-soft);
    color: var(--text-3);
    font-size: 11.5px;
  }
</style>

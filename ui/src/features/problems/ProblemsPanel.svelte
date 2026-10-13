<script lang="ts">
  // The Problems tool, a bottom panel: the diagnostics of the whole changeset in one list, the ones
  // the change introduced first, each with the review state of the hunk it sits in. A click shows a
  // problem without taking the focus, Enter and a double click jump to its source.
  import { tick, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { Unavailable } from "../../generated/Unavailable";
  import { limitedMessage, onlyStoodIn } from "../chrome";
  import BottomToolHeader from "../../lib/ui/BottomToolHeader.svelte";
  import FilterField from "../../lib/ui/FilterField.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import SeverityIcon from "../../lib/ui/SeverityIcon.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import Popover from "../../lib/ui/Popover.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import { changeset } from "../../state/changeset.svelte";
  import {
    type Grouping,
    type Level,
    type Problem,
    type Row,
    NO_FILTERS,
    emptyOf,
    filtered,
    rowsOf,
  } from "../../state/problemRows";
  import { problems } from "../../state/problems.svelte";
  import { progress as tasks } from "../../state/progress.svelte";
  import { serverList } from "../../state/serverList.svelte";
  import { languageOf, serverLine } from "../../state/serverStatus";
  import { semantic } from "../../state/semantic.svelte";
  import { versions } from "../../state/versions.svelte";

  let {
    openFiles,
    currentFile,
    onshow,
    onfile,
    oneditor,
    oninstall,
    onclose,
  }: {
    /** The files with a tab open, and the file of the tab on screen. */
    openFiles: readonly string[];
    currentFile?: string;
    /** Shows a problem; `jump` is Jump to Source. */
    onshow: (problem: Problem, jump: boolean) => void;
    /** A file's header was chosen: the Changes tab shows the file. */
    onfile: (path: string) => void;
    /** Gives the keyboard focus back to the editor. */
    oneditor: () => void;
    /** Opens Settings at the language servers, to install the one a file lacks. */
    oninstall: () => void;
    /** Hides the tool. */
    onclose: () => void;
  } = $props();

  type Scope = "changed" | "open" | "file";
  type Item =
    | Row
    | { kind: "wait"; key: string; server: string; files: string[] }
    | { kind: "na"; key: string; open: boolean; count: number }
    | { kind: "naFile"; key: string; path: string; why: Unavailable };

  const LEVELS: { id: Level; title: string }[] = [
    { id: "error", title: "Errors" },
    { id: "warning", title: "Warnings" },
    { id: "info", title: "Information and hints" },
  ];
  const ORIGIN_TITLE = {
    new: "Introduced by this change",
    existing: "The base has it too",
    pending: "Comparing with the base",
    unknown: "",
  } as const;
  /** How a problem is matched with the base's, which is why one that moved or changed reads as it does. */
  const MATCHING =
    "Matched with the base's problems by line (as the diff moves it) and message: one that only moved is existing, one whose message changed with the code reads as new.";
  const REVIEWED = {
    seen: "Seen",
    old: "Seen against an old base",
    unseen: "Not seen",
    none: "Outside any hunk",
  } as const;

  let scope = $state<Scope>("changed");
  let filters = $state({ ...NO_FILTERS, levels: { ...NO_FILTERS.levels } });
  let grouping = $state<Grouping>("origin");
  let folded = $state.raw<ReadonlySet<string>>(new Set());
  let naOpen = $state(false);
  let limitedOpen = $state(false);
  let cursor = $state(-1);
  let focused = $state(false);
  let root = $state<HTMLElement>();
  let body = $state<HTMLElement>();
  let list = $state<VirtualRows>();
  let field = $state<HTMLInputElement>();

  const scopePaths = $derived(
    scope === "changed" ? problems.changed : scope === "open" ? [...openFiles] : currentFile ? [currentFile] : [],
  );
  // The files with a tab open are asked about first, even where the change leaves them alone.
  $effect(() => {
    const open = [...openFiles];
    untrack(() => problems.want(open));
  });

  const all = $derived(problems.problemsOf(scopePaths));
  const origins = $derived(problems.originsOf(scopePaths.filter((p) => problems.served(p))));
  /** No origin is pending: each problem is new, existing or of unknown origin. */
  const settled = $derived(origins !== "pending");
  const active = $derived({ ...filters, origin: problems.origin });
  const shown = $derived(filtered(all, active, settled));
  const rows = $derived(
    rowsOf(shown, { grouping: grouping === "origin" && !settled ? "file" : grouping, settled, folded }),
  );
  const missing = $derived(problems.notAnalysed(scopePaths));
  const levelCounts = $derived({
    error: all.filter((p) => p.level === "error").length,
    warning: all.filter((p) => p.level === "warning").length,
    info: all.filter((p) => p.level === "info").length,
  });
  const reach = $derived(problems.progressOf(scopePaths));
  const stale = $derived(problems.staleIn(scopePaths));
  const waiting = $derived(reach.total > 0 && reach.answered === 0);
  const everyFileUnserved = $derived(scopePaths.length > 0 && reach.total === 0);
  const unanswered = $derived(problems.unanswered(scopePaths));
  const empty = $derived(
    everyFileUnserved ? undefined : emptyOf({ all, shown, filters: active, settled, unanswered: unanswered.length }),
  );
  const scopeName = $derived({ changed: "the changed files", open: "open files", file: "this file" }[scope]);
  /** No file can be analysed, so what is not analysed is all there is to say and is shown open. */
  const naShown = $derived(naOpen || everyFileUnserved);
  /** The server of the first file in scope, for what the notices say. */
  const serverOf = $derived.by(() => {
    const path = scopePaths.find((p) => languageOf(p, serverList.servers) !== undefined);
    const language = path && languageOf(path, serverList.servers);
    return serverList.servers.find((s) => s.language === language);
  });
  const indexing = $derived.by(() => {
    const path = scopePaths.find((p) => languageOf(p, serverList.servers) !== undefined);
    return path ? serverLine(path, serverList.servers, tasks.tasks, semantic.summary(path))?.indexing : undefined;
  });
  const items = $derived<Item[]>([
    ...rows,
    ...(indexing && serverOf && unanswered.length
      ? [{ kind: "wait" as const, key: "wait", server: serverOf.server, files: unanswered }]
      : []),
    ...(missing.length ? [{ kind: "na" as const, key: "na", open: naShown, count: missing.length }] : []),
    ...(naShown
      ? missing.map((m) => ({ kind: "naFile" as const, key: `na:${m.path}`, path: m.path, why: m.why }))
      : []),
  ]);

  const limited = $derived(changeset.restoreFailures.length ? limitedMessage(changeset.restoreFailures) : undefined);
  const latest = $derived(versions.items.at(-1)?.n);
  const lensNote = $derived.by(() => {
    const code = latest === undefined ? "the latest code" : `the latest code (v${latest})`;
    if (changeset.shownCommit !== undefined) {
      const id = changeset.commits[changeset.shownCommit]?.commit.shortId;
      return `Showing the problems of ${code}. The commit on view${id ? `, ${id},` : ""} is not analysed.`;
    }
    return changeset.range ? `Showing the problems of ${code}. The range on view is not analysed.` : undefined;
  });
  const unknownWhy = $derived(problems.unknownWhy(scopePaths));
  const unknownFiles = $derived(problems.unknownFiles(scopePaths).length);
  const baseId = $derived(changeset.facts?.base.commit.shortId);

  const keyOf = (item: Item) => (item.kind === "problem" ? `p:${item.problem.id}` : item.key);

  function fold(key: string) {
    const next = new SvelteSet(folded);
    if (!next.delete(key)) next.add(key);
    folded = next;
  }

  /** The item's own action: show a problem, fold a group, show a file. */
  function single(item: Item) {
    if (item.kind === "problem") {
      problems.selected = item.problem.id;
      onshow(item.problem, false);
    } else if (item.kind === "naFile" || item.kind === "wait") {
      return;
    } else if (item.kind === "na") {
      naOpen = !naOpen;
    } else {
      fold(item.key);
      if (item.kind === "file") onfile(item.path);
    }
  }

  function open(item: Item) {
    if (item.kind === "problem") {
      problems.selected = item.problem.id;
      onshow(item.problem, true);
    } else single(item);
  }

  const nav = new ListNav<Item>({
    element: () => body,
    items: () => items,
    single,
    double: open,
    highlight: { get: () => cursor, set: (i) => (cursor = i) },
    reveal: (i) => void list?.reveal(i),
  });
  $effect(() => () => nav.cancel());

  const plain = (e: KeyboardEvent) => !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey;

  function onkeydown(e: KeyboardEvent) {
    const take = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    if (e.target instanceof HTMLInputElement) {
      if (e.key === "Escape") {
        take();
        filters.query = "";
        body?.focus();
      } else if (e.key === "ArrowDown") {
        take();
        body?.focus();
        cursor = Math.min(items.length - 1, Math.max(cursor, -1) + 1);
        follow();
      }
      return;
    }
    const item = items.at(cursor);
    if (item && plain(e) && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      take();
      const holds = (r: Item) => r.kind === "group" || r.kind === "file" || r.kind === "na";
      const isOpen = (r: Item) => (r.kind === "na" ? naShown : "open" in r && r.open);
      if (e.key === "ArrowRight") {
        if (holds(item) && !isOpen(item)) single(item);
      } else if (holds(item) && isOpen(item)) single(item);
      else if (item.kind === "problem" || item.kind === "naFile" || item.kind === "wait") {
        let holder = cursor - 1;
        while (holder >= 0 && !holds(items[holder])) holder--;
        if (holder >= 0) {
          cursor = holder;
          list?.reveal(holder);
        }
      }
      return;
    }
    if (e.key === "Escape" && plain(e)) {
      take();
      if (filters.query) filters.query = "";
      else oneditor();
      return;
    }
    if (e.key.length === 1 && plain(e) && e.key !== " ") {
      field?.focus();
      return;
    }
    if (nav.keydown(e)) {
      take();
      if (e.key === "ArrowUp" || e.key === "ArrowDown") follow();
    }
  }

  /** The editor follows the problem the highlight is on, without taking the focus. */
  function follow() {
    const item = items.at(cursor);
    if (item?.kind !== "problem") return;
    problems.selected = item.problem.id;
    onshow(item.problem, false);
  }

  function focusin() {
    focused = true;
    if (cursor < 0) cursor = items.findIndex((i) => i.kind === "problem" && i.problem.id === problems.selected);
  }

  function focusout(e: FocusEvent) {
    if (e.relatedTarget instanceof Node && root?.contains(e.relatedTarget)) return;
    focused = false;
  }

  function click(e: MouseEvent, index: number) {
    nav.click(e, index);
  }

  function clearFilters() {
    filters = { ...NO_FILTERS, levels: { ...NO_FILTERS.levels } };
    problems.origin = "all";
  }

  function showMissing() {
    naOpen = true;
    void tick().then(() => {
      list?.reveal(items.length - 1);
    });
  }

  /** `message` with its backticked names as code. */
  const parts = (message: string) => message.split(/`([^`]+)`/).map((text, i) => ({ text, code: i % 2 === 1 }));
  const plainText = (message: string) => message.replaceAll("`", "");
  const place = (p: Problem, withFile: boolean) => `${withFile ? `${p.path.split("/").pop()}` : ""}:${p.line + 1}`;
  const tip = (p: Problem) =>
    [
      plainText(p.diagnostic.message),
      [p.diagnostic.source, p.diagnostic.code].filter(Boolean).join(" "),
      `${p.path}:${p.line + 1}`,
      ORIGIN_TITLE[p.origin],
      p.origin === "new" || p.origin === "existing" ? MATCHING : "",
    ]
      .filter(Boolean)
      .join("\n");
  const dir = (path: string) => path.slice(0, path.lastIndexOf("/") + 1);
  const base = (path: string) => path.slice(path.lastIndexOf("/") + 1);
</script>

<section aria-label="Problems" bind:this={root} onfocusin={focusin} onfocusout={focusout}>
  <BottomToolHeader title="Problems" count={problems.summary?.badge ?? 0} {onclose}>
    {#snippet controls()}
      <Segmented
        small
        label="Scope"
        value={scope}
        onchange={(s) => {
          scope = s;
        }}
        options={[
          { id: "changed", label: "Changed files" },
          { id: "open", label: "Open files" },
          { id: "file", label: "This file" },
        ]}
      />
      {#each LEVELS as level (level.id)}
        <button
          class="flat chip mono"
          type="button"
          title={level.title}
          aria-pressed={filters.levels[level.id]}
          onclick={() => (filters.levels[level.id] = !filters.levels[level.id])}
        >
          <SeverityIcon severity={level.id} />
          {levelCounts[level.id]}
        </button>
      {/each}
      <Segmented
        small
        label="Origin"
        value={problems.origin}
        onchange={(o) => {
          problems.origin = o;
        }}
        options={[
          {
            id: "new",
            label: "New",
            disabled: !settled,
            title: settled ? "Only problems this change introduced" : "Not known yet",
          },
          { id: "all", label: "All" },
        ]}
      />
      <span class="grow"></span>
      <FilterField bind:input={field} bind:value={filters.query} label="Filter problems" {onkeydown} />
      <Popover align="right">
        {#snippet trigger(toggle)}
          <button
            class="flat menu"
            class:on={filters.unseenOnly || filters.changedOnly || grouping !== "origin"}
            type="button"
            title="Group and filter"
            aria-label="Group and filter"
            onclick={toggle}><Icon name="filter" size={14} /></button
          >
        {/snippet}
        <div class="menu-sheet">
          <h5>Group by</h5>
          {#each [{ id: "origin", label: "Origin, then file" }, { id: "file", label: "File" }, { id: "severity", label: "Severity" }] as g (g.id)}
            <label class:off={g.id === "origin" && !settled}>
              <input
                type="radio"
                name="problems-group"
                checked={grouping === g.id}
                disabled={g.id === "origin" && !settled}
                onchange={() => (grouping = g.id as Grouping)}
              />
              {g.label}
            </label>
          {/each}
          <h5>Show only</h5>
          <label><input type="checkbox" bind:checked={filters.unseenOnly} /> Problems in hunks not seen</label>
          <label><input type="checkbox" bind:checked={filters.changedOnly} /> Problems on changed lines</label>
        </div>
      </Popover>
    {/snippet}
  </BottomToolHeader>

  {#if indexing && serverOf}
    <div class="notice info">
      <SeverityIcon severity="info" />
      <div>
        {serverOf.server} is indexing{indexing.fraction === undefined
          ? ""
          : ` (${Math.round(indexing.fraction * 100)}%)`}. Problems from the other files, and any that need the whole
        project, may be missing until it is done.
        {#if indexing.fraction !== undefined}
          <div class="bar"><i style:width="{indexing.fraction * 100}%"></i></div>
        {/if}
      </div>
    </div>
  {/if}
  {#if limited}
    <div class="notice warn">
      <SeverityIcon severity="warning" />
      <div>
        {limited}{onlyStoodIn(changeset.restoreFailures) ? "" : " Errors about unresolved names may not be real."}
        <button class="link" type="button" aria-expanded={limitedOpen} onclick={() => (limitedOpen = !limitedOpen)}
          >{limitedOpen ? "Hide details" : "Details"}</button
        >
        {#if limitedOpen}
          <ul class="failures">
            {#each changeset.restoreFailures as failure (failure.project + failure.tool + failure.message)}
              <li><strong>{failure.project}</strong> ({failure.tool}): <code>{failure.message}</code></li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>
  {/if}
  {#if lensNote}
    <div class="notice info">
      <SeverityIcon severity="info" />
      <div>{lensNote}</div>
    </div>
  {/if}
  {#if origins === "pending" && !waiting && !everyFileUnserved && reach.answered > 0}
    <div class="notice info">
      <SeverityIcon severity="info" />
      <div>Comparing with the base to tell new problems from existing ones. They are listed by file until then.</div>
    </div>
  {/if}
  {#if origins === "unknown"}
    <div class="notice warn">
      <SeverityIcon severity="warning" />
      <div>
        Cannot tell which problems are new{unknownFiles > 1 ? ` in ${unknownFiles} files` : ""}: {unknownWhy
          ? unknownWhy.message
          : "the base could not be analysed"}, so none of {unknownFiles > 1 ? "theirs is" : "that file's is"} marked new.
        <button class="link" type="button" onclick={() => problems.retry()}>Retry</button>
      </div>
    </div>
  {/if}

  <!-- The list takes the keys of its rows, which are not in the tab order. -->
  <div class="body" class:stale role="presentation" bind:this={body} {onkeydown} {...nav.attributes}>
    {#if waiting && !items.length}
      <div class="empty">
        <Spinner />
        <b>{serverOf ? `${serverOf.server} is starting` : "The language server is starting"}</b>
        <span class="sub">Problems appear as soon as it answers. This takes a moment after opening a project.</span>
      </div>
    {:else if everyFileUnserved && !missing.length}
      <div class="empty"><b>Nothing to analyse here</b></div>
    {:else if empty?.kind === "none"}
      <div class="empty">
        <span class="ok"><Icon name="check" size={22} stroke={1.8} /></span>
        <b>No problems in {scopeName}</b>
        {#if reach.answered}
          <span class="sub">{reach.answered} analysed {reach.answered === 1 ? "file" : "files"} checked.</span>
        {/if}
      </div>
    {:else if empty?.kind === "partial"}
      <div class="empty">
        <b>No problems so far in {scopeName}</b>
        <span class="sub">{reach.answered} of {reach.total} files analysed.</span>
      </div>
    {:else if empty?.kind === "onlyExisting"}
      <div class="empty">
        <b>No new problems in {scopeName}</b>
        <span class="sub"
          >{empty.existing} existing ·
          <button class="link" type="button" onclick={() => (problems.origin = "all")}>Show all</button></span
        >
      </div>
    {:else if empty?.kind === "hidden"}
      <div class="empty">
        <b>{empty.hidden} {empty.hidden === 1 ? "problem" : "problems"} hidden by the filters</b>
        <span class="sub"><button class="link" type="button" onclick={clearFilters}>Clear filters</button></span>
      </div>
    {:else if everyFileUnserved}
      <div class="empty">
        <b>No language support for these files</b>
        <span class="sub"
          >Vet shows problems that a language server reports. None is available for {missing.length} of the changed files,
          so none is analysed.</span
        >
      </div>
    {/if}
    {#if items.length}
      <VirtualRows bind:this={list} count={items.length} key={(i) => (items[i] ? keyOf(items[i]) : "")}>
        {#snippet row(i)}
          {@const item = items[i]}
          {#if item}
            {#if item.kind === "group"}
              <button
                class="flat line group"
                class:cur={focused && i === cursor}
                class:new={item.tone === "new"}
                type="button"
                tabindex="-1"
                onclick={(e) => click(e, i)}
              >
                <span class="chev"><Icon name={item.open ? "open" : "closed"} size={10} /></span>
                {#if item.tone === "error" || item.tone === "warning" || item.tone === "info"}
                  <SeverityIcon severity={item.tone} />
                {/if}
                <span>{item.title}</span>
                <span class="n" class:strong={item.tone === "new"}>{item.count}</span>
              </button>
            {:else if item.kind === "file"}
              <button
                class="flat line filehead"
                class:cur={focused && i === cursor}
                type="button"
                tabindex="-1"
                onclick={(e) => click(e, i)}
              >
                <span class="chev"><Icon name={item.open ? "open" : "closed"} size={10} /></span>
                <FileIcon name={base(item.path)} />
                <span class="path"><span class="dir">{dir(item.path)}</span>{base(item.path)}</span>
                <span class="n mono">{item.count}</span>
              </button>
            {:else if item.kind === "problem"}
              {@const p = item.problem}
              <button
                class="flat line problem"
                class:cur={focused && i === cursor}
                class:sel={p.id === problems.selected}
                class:old={p.origin === "existing"}
                class:inset={item.showFile}
                type="button"
                tabindex="-1"
                title={tip(p)}
                onclick={(e) => click(e, i)}
              >
                <SeverityIcon severity={p.level} />
                <span class="msg"
                  >{#each parts(p.diagnostic.message) as part, n (n)}{#if part.code}<code>{part.text}</code
                      >{:else}{part.text}{/if}{/each}</span
                >
                <span class="src mono">{[p.diagnostic.source, p.diagnostic.code].filter(Boolean).join(" ")}</span>
                {#if item.pill && (p.origin === "new" || p.origin === "existing" || p.origin === "pending")}
                  <span class="pill {p.origin}" title={ORIGIN_TITLE[p.origin]}
                    >{p.origin === "pending" ? "comparing" : p.origin}</span
                  >
                {/if}
                <span class="rv {p.reviewed}" title={REVIEWED[p.reviewed]}><i></i></span>
                <span class="loc mono">{place(p, item.showFile)}</span>
              </button>
            {:else if item.kind === "wait"}
              <div class="line waiting">
                <Spinner />
                <span class="path">Waiting for {item.server}: {item.files.map(base).join(", ")}</span>
              </div>
            {:else if item.kind === "na"}
              <button
                class="flat line group"
                class:cur={focused && i === cursor}
                type="button"
                tabindex="-1"
                onclick={(e) => click(e, i)}
              >
                <span class="chev"><Icon name={item.open ? "open" : "closed"} size={10} /></span>
                <span>Not analysed</span>
                <span class="n">{item.count}</span>
              </button>
            {:else}
              <div class="line na" class:cur={focused && i === cursor}>
                <FileIcon name={base(item.path)} />
                <span class="path"><span class="dir">{dir(item.path)}</span>{base(item.path)}</span>
                <span class="why">{item.why.label}</span>
                {#if item.why.kind === "serverMissing"}
                  <button class="link" type="button" onclick={oninstall}>Install…</button>
                {/if}
              </div>
            {/if}
          {/if}
        {/snippet}
      </VirtualRows>
    {/if}
  </div>

  <div class="stat">
    {#if stale}
      <Spinner />
      <span>Updating{latest === undefined ? "" : ` for v${latest}`} · {reach.answered} of {reach.total} files</span>
    {:else}
      <span>{reach.answered} of {reach.total} files analysed</span>
    {/if}
    {#if missing.length && !everyFileUnserved}
      <span>·</span>
      <button class="link" type="button" onclick={showMissing}>{missing.length} not analysed</button>
    {/if}
    <span class="grow"></span>
    {#if origins === "known" && baseId}<span>origin: compared with base <span class="mono">{baseId}</span></span>{/if}
  </div>
</section>

<style>
  section {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--bg-panel);
    outline: none;
  }
  .grow {
    flex: 1;
  }
  .chip {
    height: 22px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 7px;
    border-radius: var(--radius);
    font-size: var(--fs-mono-s);
    color: var(--text-2);
  }
  .chip[aria-pressed="true"] {
    background: var(--toggle);
    color: var(--text);
  }
  .chip[aria-pressed="false"] {
    opacity: 0.55;
  }
  .chip[aria-pressed="false"]:hover {
    opacity: 0.9;
  }
  .menu {
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    color: var(--text-2);
  }
  .menu.on {
    background: var(--toggle);
    color: var(--accent);
  }
  .menu-sheet {
    width: 250px;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-ctl);
  }
  .menu-sheet h5 {
    margin: 4px 4px 2px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    font-weight: 600;
  }
  .menu-sheet label {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 4px;
    border-radius: 4px;
  }
  .menu-sheet label:hover {
    background: var(--hov);
  }
  .menu-sheet label.off {
    opacity: 0.5;
  }
  .notice {
    flex: none;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 12px;
    font-size: 12px;
    color: var(--text-2);
    border-bottom: 1px solid var(--line-soft);
  }
  .notice :global(.severity-icon) {
    margin-top: 2px;
  }
  .notice.warn {
    background: color-mix(in srgb, var(--severity-warning) 12%, var(--bg-panel));
  }
  .notice.info {
    background: color-mix(in srgb, var(--severity-info) 9%, var(--bg-panel));
  }
  .bar {
    margin-top: 5px;
    height: 3px;
    width: 160px;
    background: var(--track);
    border-radius: 2px;
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: var(--accent);
    cursor: default;
  }
  .link:hover {
    text-decoration: underline;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
    outline: none;
  }
  .body.stale {
    opacity: 0.55;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 22px 24px;
    text-align: center;
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .empty b {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
  }
  .empty .sub {
    color: var(--text-3);
    max-width: 46ch;
  }
  .ok {
    color: var(--ok);
    display: inline-flex;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--h-row);
    width: 100%;
    box-sizing: border-box;
    padding: 0 12px;
    text-align: left;
    white-space: nowrap;
    font-size: var(--fs-ctl);
    border-radius: 0;
  }
  .group {
    padding-left: 12px;
    font-weight: 600;
    color: var(--text-2);
  }
  .group.new {
    color: var(--text);
  }
  .filehead {
    padding-left: 22px;
    color: var(--text);
  }
  .chev {
    flex: none;
    width: var(--w-chev);
    display: inline-flex;
    justify-content: center;
    color: var(--text-2);
  }
  .dir {
    color: var(--text-3);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .n {
    flex: none;
    font-weight: 400;
    color: var(--text-3);
    font-size: 11.5px;
  }
  .n.strong {
    color: var(--accent);
    font-weight: 600;
  }
  .problem {
    padding-left: 44px;
    color: var(--text);
  }
  .problem.inset {
    padding-left: 22px;
  }
  .problem.old {
    color: var(--text-2);
  }
  .problem.old :global(.severity-icon) {
    opacity: 0.8;
  }
  .msg {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .msg code {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .src,
  .loc {
    flex: none;
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .loc {
    min-width: 40px;
    text-align: right;
  }
  .pill {
    flex: none;
    display: inline-flex;
    align-items: center;
    height: 15px;
    padding: 0 6px;
    border-radius: 8px;
    font-size: 10.5px;
    font-weight: 600;
    line-height: 1;
  }
  .pill.new {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .pill.existing {
    color: var(--text-3);
    border: 1px solid var(--line);
    font-weight: 400;
  }
  .pill.pending {
    color: var(--text-3);
    border: 1px dashed var(--line);
    font-weight: 400;
  }
  .rv {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
  }
  .rv i {
    display: block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
  .rv.seen i {
    background: var(--seen);
  }
  .rv.old i {
    background: var(--old-base-stripes);
  }
  .rv.unseen i {
    border: 1.5px solid var(--text-3);
    box-sizing: border-box;
  }
  .rv.none i {
    width: 7px;
    height: 1.5px;
    border-radius: 1px;
    background: var(--text-3);
    opacity: 0.7;
  }
  .sel {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  section:focus-within .sel {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .cur {
    background: var(--select);
  }
  .na {
    padding-left: 34px;
    color: var(--text-2);
  }
  .waiting {
    color: var(--text-3);
  }
  .failures {
    margin: 4px 0 0;
    padding-left: 16px;
  }
  .failures code {
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: pre-wrap;
  }
  .why {
    flex: none;
    color: var(--text-3);
  }
  .stat {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 24px;
    padding: 0 12px;
    border-top: 1px solid var(--line-soft);
    font-size: 11.5px;
    color: var(--text-3);
  }
</style>

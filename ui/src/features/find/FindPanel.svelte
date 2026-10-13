<script lang="ts">
  // The Find tool, a bottom panel: the result sets of Find in Files and Find usages as tabs, each
  // listing its places by file with the review state of the hunk each is in, and a preview of the
  // place chosen. A click chooses a place, Enter and a double click open it where it was found.
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { letterOfStatus } from "../../lib/changeLetter";
  import ChangeLetter from "../../lib/ui/ChangeLetter.svelte";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import BottomToolHeader from "../../lib/ui/BottomToolHeader.svelte";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import FilterField from "../../lib/ui/FilterField.svelte";
  import Banner from "../../lib/ui/Banner.svelte";
  import FilterChip from "../../lib/ui/FilterChip.svelte";
  import GroupHeader from "../../lib/ui/GroupHeader.svelte";
  import ReviewDot from "../../lib/ui/ReviewDot.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import type { Backend } from "../../state/backend";
  import { type FindSet, findSets } from "../../state/findSets.svelte";
  import { type Item, narrowing, itemsOf, visible } from "../../state/findRows";
  import { previewBasis, relocated } from "../../state/findRuns";
  import type { Place } from "../../state/foundPlace";
  import { basename, dirname } from "../../lib/paths";
  import { MODULE_LEVEL } from "../../lib/symbolLabel";
  import { ResultPreview } from "../search";
  import FindTab from "./FindTab.svelte";
  import { countText, cutText, rerunText, staleText } from "./findWords";

  let {
    backend,
    onopen,
    oneditor,
    onclose,
  }: {
    backend: Backend;
    /** Hides the tool. */
    onclose: () => void;
    /** Opens a place where it was found; `ownTab` asks for the file's own tab. */
    onopen: (set: FindSet, place: Place, ownTab: boolean) => void;
    /** Gives the keyboard focus back to the editor. */
    oneditor: () => void;
  } = $props();

  const set = $derived(findSets.current);
  const shown = $derived(set ? visible(set.places, set.filters) : []);
  const items = $derived(set ? itemsOf(shown, set.folded) : []);
  const cursor = $derived(items.findIndex((i) => i.key === set?.selected));
  const chosen = $derived.by(() => {
    const item = items.at(cursor);
    return item?.kind === "place" ? item.place : undefined;
  });
  const stale = $derived(!!set && findSets.stale(set));
  const moved = $derived<ReadonlySet<string>>(set && stale ? findSets.changedFiles(set) : new Set());
  const files = $derived(new Set(shown.map((p) => p.file.path)).size);
  const notSeen = $derived(shown.filter((p) => p.review === "unseen" || p.review === "old").length);
  const cut = $derived(
    set?.spec.type === "usages" && set.phase !== "running" && set.total > set.places.length ? set.total : undefined,
  );
  const version = $derived(set?.ran.version);

  let body = $state<HTMLElement>();
  let list = $state<VirtualRows>();
  let field = $state<HTMLInputElement>();

  const select = (item: Item | undefined) => {
    if (set && item) set.selected = item.key;
  };

  function fold(key: string) {
    if (!set) return;
    const next = new SvelteSet(set.folded);
    if (!next.delete(key)) next.add(key);
    set.folded = next;
  }

  /** A place's own action is to be chosen, a file's to fold. */
  function single(item: Item) {
    select(item);
    if (item.kind === "file") fold(item.key);
  }

  function open(item: Item, ownTab = false) {
    if (item.kind === "place" && set) {
      select(item);
      onopen(set, relocated(item.place, set.moves.get(item.place.id)), ownTab);
    } else single(item);
  }

  const nav = new ListNav<Item>({
    element: () => body,
    items: () => items,
    single,
    double: (item) => open(item),
    highlight: { get: () => cursor, set: (i) => select(items[i]) },
    reveal: (i) => void list?.reveal(i),
  });
  $effect(() => () => nav.cancel());

  const plain = (e: KeyboardEvent) => !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey;

  function onkeydown(e: KeyboardEvent) {
    const take = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "r" && set) {
      take();
      findSets.rerun(set);
      return;
    }
    if (e.target instanceof HTMLInputElement) {
      if (e.key === "Escape" && set) {
        take();
        set.filters.query = "";
        body?.focus();
      } else if (e.key === "ArrowDown" || e.key === "Enter") {
        take();
        body?.focus();
        if (cursor < 0) select(items.find((i) => i.kind === "place"));
      }
      return;
    }
    const item = items.at(cursor);
    if (item && set && plain(e) && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      take();
      if (item.kind === "file") {
        if (item.open === (e.key === "ArrowLeft")) fold(item.key);
      } else if (e.key === "ArrowLeft") {
        // A place steps back to its file's header.
        let header = cursor;
        while (header > 0 && items[header].kind !== "file") header--;
        select(items[header]);
        list?.reveal(header);
      }
      return;
    }
    if (e.key === "Enter" && e.altKey && !e.ctrlKey && !e.metaKey && item) {
      take();
      open(item, true);
      return;
    }
    if (e.key === "Escape" && plain(e)) {
      take();
      if (set && narrowing({ ...set.filters, changeset: false, notSeen: false })) set.filters.query = "";
      else oneditor();
      return;
    }
    if (e.key.length === 1 && plain(e) && e.key !== " ") {
      field?.focus();
      return;
    }
    if (nav.keydown(e)) take();
  }

  // The places of a set that went stale are asked for where they are now.
  $effect(() => {
    const current = set;
    const revision = findSets.revision;
    if (current && stale && revision !== undefined) untrack(() => void findSets.locate(current));
  });

  // A set's chosen place is revealed when the tab is shown again.
  $effect(() => {
    const at = cursor;
    untrack(() => {
      if (at >= 0) list?.reveal(at);
    });
  });

  const countOf = (s: FindSet) => (s.hasRun ? visible(s.places, s.filters).length : undefined);
  const dir = (path: string) => (dirname(path) ? `${dirname(path)}/` : "");
  const clearFilters = () => {
    if (!set) return;
    set.filters.query = "";
    set.filters.changeset = false;
    set.filters.notSeen = false;
  };
  const chipLabel = (s: FindSet) =>
    s.spec.type === "search" && s.spec.scope.commit
      ? s.spec.scope.commit.shortId
      : `${s.spec.type === "search" ? s.spec.scope.label : "All changes"}${s.ran.version !== undefined && (s.spec.type === "usages" || s.spec.scope.label === "All changes") ? ` v${s.ran.version}` : ""}`;
</script>

<section aria-label="Find" data-filter-keys>
  <BottomToolHeader title="Find" count={set && countOf(set)} {onclose}>
    {#snippet controls()}
      {#if set}
        <span class:hot={stale}>
          {#if set.phase === "running"}
            <IconButton icon="stop" label="Stop" size={22} iconSize={12} onclick={() => findSets.stop(set)} />
          {:else}
            <IconButton
              icon="refresh"
              label="Rerun"
              title="Rerun  {navigator.platform.includes('Mac') ? '⌘' : 'Ctrl+'}R"
              size={22}
              iconSize={14}
              onclick={() => findSets.rerun(set)}
            />
          {/if}
        </span>
        <FilterChip
          title="Only places in the changeset"
          on={set.filters.changeset}
          onclick={() => (set.filters.changeset = !set.filters.changeset)}>Changeset</FilterChip
        >
        <FilterChip
          title="Only places in hunks not seen"
          on={set.filters.notSeen}
          onclick={() => (set.filters.notSeen = !set.filters.notSeen)}>Not seen</FilterChip
        >
        <span class="grow"></span>
        <span
          class="view"
          title={set.spec.type === "search" && set.spec.scope.commit
            ? "Reads the text as that commit has it"
            : "Line numbers are of the text the set ran on"}
        >
          {#if set.commit}<Icon name="commit" size={12} />{/if}
          <span class:mono={set.commit}>{chipLabel(set)}</span>
        </span>
        <FilterField bind:input={field} bind:value={set.filters.query} label="Filter results" {onkeydown} />
        <IconButton
          icon="preview"
          label="Preview of the code"
          size={22}
          iconSize={14}
          active={findSets.preview}
          onclick={() => (findSets.preview = !findSets.preview)}
        />
      {/if}
    {/snippet}
  </BottomToolHeader>
  <div class="strip" role="tablist" aria-label="Result sets">
    {#each findSets.sets as s (s.id)}
      <FindTab
        set={s}
        selected={s.id === findSets.active}
        count={countOf(s)}
        stale={findSets.stale(s)}
        onselect={() => (findSets.active = s.id)}
        onpin={() => findSets.togglePin(s)}
        onclose={() => findSets.close(s.id)}
      />
    {/each}
  </div>

  {#if set}
    {#if stale}
      <Banner message={staleText([...moved])} action={{ label: "Rerun", onclick: () => findSets.rerun(set) }} />
    {:else if set.rerun && set.phase === "done"}
      <Banner message={rerunText(set.rerun)} />
    {/if}
    {#if cut !== undefined}
      <Banner
        message="{cutText(set.places.length, cut)} Filter the list or show all."
        action={{ label: "Show all", onclick: () => findSets.showAll(set) }}
      />
    {:else if set.truncated && set.phase === "done"}
      <Banner message="The search stopped at its limit of matches. Narrow the query to read the rest." />
    {/if}

    <div class="main">
      <!-- The list takes the keys of its rows, which are not in the tab order. -->
      <div class="list" role="presentation" bind:this={body} {onkeydown} {...nav.attributes}>
        {#if set.phase === "running" && !set.hasRun}
          <EmptyState
            busy
            title={set.spec.type === "usages" ? `Finding usages of ${set.title}` : `Searching for ${set.title}`}
          >
            {#snippet actions()}
              <button class="ctl" type="button" onclick={() => findSets.stop(set)}>Stop</button>
            {/snippet}
          </EmptyState>
        {:else if set.phase === "failed" || set.phase === "stopped"}
          <EmptyState
            icon="warning"
            tone="warn"
            title={set.phase === "failed" ? "The search failed" : "No result"}
            text={set.message}
          >
            {#snippet actions()}
              <button class="ctl" type="button" onclick={() => findSets.rerun(set)}>Rerun</button>
            {/snippet}
          </EmptyState>
        {:else if !set.places.length}
          <EmptyState
            title={set.spec.type === "usages" ? "No usages" : "No matches"}
            text={set.spec.type === "usages"
              ? `Nothing refers to ${set.title} in the code the language servers know.`
              : `${set.title} does not occur where it was searched.`}
          />
        {:else if !items.length}
          <EmptyState title="No results match">
            {set.filters.query.trim()
              ? `No result contains "${set.filters.query.trim()}".`
              : "The filters hide every result."}
            {set.places.length} kept
            {set.places.length === 1 ? "result is" : "results are"} hidden.
            <button class="ctl" type="button" onclick={clearFilters}>Remove filters</button>
          </EmptyState>
        {:else}
          <VirtualRows bind:this={list} count={items.length} key={(i) => items.at(i)?.key ?? ""}>
            {#snippet row(i)}
              {@const item = items.at(i)}
              {#if item?.kind === "file"}
                {@const f = item.file}
                <GroupHeader
                  kind="file"
                  open={item.open}
                  count={item.count}
                  cur={i === cursor}
                  on={i === cursor}
                  tip={f.path}
                  tabindex={-1}
                  onclick={(e) => nav.click(e, i)}
                >
                  {#snippet before()}
                    {#if f.external}<span class="lib"><Icon name="module" size={12} /></span>{:else}<FileIcon
                        name={basename(f.path)}
                      />{/if}
                  {/snippet}
                  <span class="named">
                    <span class="path"
                      >{#if f.external}{f.label}{:else}<span class="dir">{dir(f.path)}</span>{basename(
                          f.path,
                        )}{/if}</span
                    >
                    {#if f.status}<ChangeLetter letter={letterOfStatus(f.status)} title={f.status} />{/if}
                  </span>
                  {#snippet trailing()}
                    {#if moved.has(f.path)}<span class="moved" title="This file changed since the run"
                        >changed since</span
                      >{/if}
                    {#if item.notSeen}<span class="uns">{item.notSeen} not seen</span>{/if}
                  {/snippet}
                </GroupHeader>
              {:else if item?.kind === "place"}
                {@const p = item.place}
                {@const move = set.moves.get(p.id)}
                {@const old = moved.has(p.file.path) && !move?.kept}
                <button
                  class="flat pick line place"
                  class:on={i === cursor}
                  class:cur={i === cursor}
                  class:old
                  class:fresh={set.fresh.has(p.id)}
                  type="button"
                  tabindex="-1"
                  title="{p.file.path}:{(move?.line ?? p.line) + 1}&#10;{p.text}{move && !move.kept
                    ? '&#10;This line was changed since the run; the row opens at the nearest line.'
                    : old
                      ? '&#10;This file changed since the run; the row opens at the line it had.'
                      : ''}"
                  onclick={(e) => nav.click(e, i)}
                >
                  <ReviewDot reviewed={p.review} />
                  <span class="ln mono">{(move?.line ?? p.line) + 1}</span>
                  <span class="within" class:none={p.within === MODULE_LEVEL} title={p.withinTitle}
                    >{p.within ?? ""}</span
                  >
                  <span class="code mono"
                    >{#each p.runs as run, r (r)}{#if run.hit}<mark class={run.className}>{run.text}</mark
                        >{:else if run.className}<span class={run.className}>{run.text}</span
                        >{:else}{run.text}{/if}{/each}</span
                  >
                  {#if p.side === "base"}<span class="tag" title="In the old code: opens on the diff's old side"
                      >base</span
                    >{/if}
                  {#if move && !move.kept}<span class="tag" title="This line was changed since the run">edited</span
                    >{/if}
                </button>
              {/if}
            {/snippet}
          </VirtualRows>
        {/if}
      </div>
      {#if findSets.preview && chosen && !(set.phase === "failed" || set.phase === "stopped")}
        {@const basis = previewBasis(chosen, moved.has(chosen.file.path))}
        <div class="preview">
          {#if moved.has(chosen.file.path)}
            <div class="pvn">
              <Icon name="warning" size={12} />The file changed since the run. This is the current text{set.moves.has(
                chosen.id,
              )
                ? ", at the line the row is now."
                : "."}
            </div>
          {/if}
          <ResultPreview
            {backend}
            file={{
              path: chosen.file.path,
              status: chosen.file.status,
              completion: chosen.file.completion,
              hunks: basis.hunks,
              states: basis.states,
              lines: set.places
                .filter((p) => p.file === chosen.file)
                .map((p) => ({
                  kind: p.kind,
                  line: set.moves.get(p.id)?.line ?? p.line,
                  text: p.text,
                  textStart: 0,
                  hits: [...p.hits],
                })),
            }}
            line={{ ...relocated(chosen, set.moves.get(chosen.id)), kind: basis.kind }}
            view={set.spec.type === "search" && set.spec.request.scope.type === "view"
              ? set.spec.request.scope.view
              : { type: "changeset" }}
            revision={findSets.revision ?? 0}
            repository={set.spec.type === "search" && set.spec.request.scope.type === "repository"}
          />
        </div>
      {/if}
    </div>

    <div class="stat">
      {#if set.phase === "running"}
        <Spinner />
        <span>{set.hasRun ? "Running again; the earlier results stay until this is done" : "Searching"}</span>
      {:else}
        <span class="hd"
          >{set.spec.type === "usages" ? "Usages of " : "Find in Files: "}<span class="mono">{set.title}</span></span
        >
        <span>·</span>
        <span>{countText(shown.length, files, notSeen)}</span>
        <span class="grow"></span>
        <span
          >{set.commit && set.spec.type === "search"
            ? `at ${set.spec.scope.commit?.shortId}`
            : version === undefined
              ? ""
              : `ran on v${version}`}</span
        >
      {/if}
    </div>
  {:else}
    <div class="nothing">
      <EmptyState icon="find" title="Nothing kept yet" text="Results of Find in Files and Find Usages stay here." />
    </div>
  {/if}
</section>

<style>
  section {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--bg-panel);
    outline: none;
  }
  .strip {
    flex: none;
    display: flex;
    align-items: stretch;
    height: var(--h-tab);
    overflow-x: auto;
    scrollbar-width: none;
    border-bottom: 1px solid var(--line-soft);
  }
  .grow {
    flex: 1;
  }
  .hot :global(button) {
    color: var(--old-base);
  }
  .view {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--line);
    border-radius: 11px;
    color: var(--text-3);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .list {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
    outline: none;
  }
  .preview {
    flex: 0 0 46%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: auto;
    border-left: 1px solid var(--line);
    background: var(--bg-editor);
  }
  .preview :global(.preview) {
    border-top: 0;
  }
  .pvn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 12px;
    font-size: 11.5px;
    color: var(--text-2);
    background: color-mix(in srgb, var(--old-base) 12%, var(--bg-editor));
    border-bottom: 1px solid var(--line-soft);
  }
  .pvn :global(svg) {
    color: var(--old-base);
  }
  .nothing {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    background: var(--bg-editor);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 7px;
    height: var(--h-row);
    width: 100%;
    box-sizing: border-box;
    padding: 0 12px 0 10px;
    text-align: left;
    white-space: nowrap;
    font-size: var(--fs-ctl);
    border-radius: 0;
    position: relative;
  }
  .lib {
    display: inline-flex;
    color: var(--text-3);
  }
  .named {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dir {
    color: var(--text-3);
  }
  .moved {
    flex: none;
    color: var(--old-base);
    font-size: 11px;
  }
  .uns {
    flex: none;
    color: var(--accent);
    font-size: 11.5px;
    font-weight: 500;
  }
  .place {
    padding-left: 30px;
  }
  .place.old .ln,
  .place.old .code {
    opacity: 0.55;
  }
  .place.fresh::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--accent);
  }
  .ln {
    flex: none;
    width: 36px;
    text-align: right;
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .within {
    flex: none;
    width: 100px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-2);
  }
  .within.none {
    color: var(--text-3);
  }
  .code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
    font-size: var(--fs-mono);
  }
  mark {
    background: var(--find);
    color: inherit;
    border-radius: 2px;
  }
  .tag {
    flex: none;
    padding: 0 5px;
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text-3);
    font-size: 10.5px;
    line-height: 14px;
  }
  .stat {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
    padding: 0 12px;
    border-top: 1px solid var(--line-soft);
    font-size: 11.5px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
  }
  .stat .hd {
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .stat .mono {
    font-size: 11px;
  }
</style>

<script lang="ts" generics="T">
  // One root of the Symbol panel's tree: a titled answer that is loading, failed or shown, which
  // opens and shuts and is remembered so. An answer that lists places shows `ROWS` of them, then
  // "N more"; a root with nothing to list is one muted line.
  import type { Snippet } from "svelte";
  import { letterOfChange } from "../../lib/changeLetter";
  import { storeFlag, storedFlag } from "../../lib/storage";
  import { onChevron } from "../../lib/ui/chevron";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import MoreRow from "../../lib/ui/MoreRow.svelte";
  import SectionHead from "../../lib/ui/SectionHead.svelte";
  import SeverityIcon from "../../lib/ui/SeverityIcon.svelte";
  import TreeRow from "../../lib/ui/TreeRow.svelte";
  import type { Load } from "../../state/symbols.svelte";
  import { SvelteSet } from "svelte/reactivity";
  import SymbolGlyph from "./SymbolGlyph.svelte";
  import { type Body, type Entry, ROWS, type Target } from "./symbolTree";

  let {
    id,
    title,
    load,
    build,
    children,
    ongo,
    warn = false,
    hint,
    find,
  }: {
    /** Names the root where its state is kept. */
    id: string;
    title: string;
    load?: Load<T>;
    /** Lists the answer's places as rows. */
    build?: (value: T) => Body;
    /** Draws the answer itself, when it lists no places. */
    children?: Snippet<[T]>;
    /** A row was clicked. */
    ongo?: (target: Target) => void;
    /** The root says something to be careful about. */
    warn?: boolean;
    /** A caveat about the answer, in a tooltip next to the title. */
    hint?: string;
    /** Opens the places in the Find window; a button in the title line. */
    find?: { label: string; onclick: () => void };
  } = $props();

  const KEY = "vet.symbols.section.";
  // The root keeps the state it was first shown with.
  // svelte-ignore state_referenced_locally
  let shown = $state(storedFlag(KEY + id, true));
  // Rows the reviewer opened or shut against what they start as, and lists asked to show all.
  const flipped = new SvelteSet<string>();
  const all = new SvelteSet<string>();

  const body = $derived(load?.state === "done" && build ? build(load.value) : undefined);

  const FATE = { bad: "var(--del-text)", warn: "var(--old-base)", mute: "var(--text-3)" } as const;

  function toggle() {
    shown = !shown;
    storeFlag(KEY + id, shown, true);
  }

  const holds = (e: Entry) => e.children.length > 0 && !e.fixed;
  const isOpen = (e: Entry) => holds(e) && flipped.has(e.key) !== (e.open ?? false);

  function fold(e: Entry) {
    if (flipped.has(e.key)) flipped.delete(e.key);
    else flipped.add(e.key);
  }

  function click(e: Entry, event: MouseEvent) {
    if (holds(e) && (onChevron(event) || !e.go)) fold(e);
    else if (e.go) ongo?.(e.go);
  }
</script>

{#snippet list(entries: readonly Entry[], depth: number, cap: string | undefined)}
  {@const rows = cap === undefined || all.has(cap) ? entries : entries.slice(0, ROWS)}
  {#each rows as e (e.key)}
    {@render row(e, depth)}
  {/each}
  {#if cap !== undefined && rows.length < entries.length}
    <MoreRow count={entries.length - rows.length} {depth} onclick={() => all.add(cap)} />
  {/if}
{/snippet}

{#snippet row(e: Entry, depth: number)}
  {@const open = isOpen(e)}
  <TreeRow
    wrap
    {depth}
    open={holds(e) ? open : undefined}
    tone={e.muted === true || e.branch === true ? "plain" : "strong"}
    gone={e.gone}
    title={e.title}
    status={e.change && letterOfChange(e.change)}
    statusTitle={e.change}
    disabled={!holds(e) && !e.go}
    onclick={(event) => click(e, event)}
  >
    {#snippet glyph()}
      {#if e.glyph === "file"}
        <span class="file"><FileIcon name={e.name} /></span>
      {:else if e.glyph}
        <SymbolGlyph kind={e.glyph} />
      {/if}
    {/snippet}
    <span class="id" class:code={!e.file && !e.branch}>{e.name}</span>{#if e.container}<span class="ctr"
        >{e.container}</span
      >{/if}{#if e.sub}<span class="sub">{e.sub}</span>{/if}
    {#snippet trailing()}
      {#if e.loc}<span class="loc">{e.loc}</span>{/if}
      {#if e.lines}<span class="ln">{e.lines}</span>{/if}
      {#if e.fate}<span class="fate" style:color={FATE[e.fate.tone]}>{e.fate.label}</span>{/if}
      {#if holds(e) && !open}<span class="n">{e.count}</span>{/if}
    {/snippet}
  </TreeRow>
  {#if e.fixed === true || open}
    {@render list(e.children, depth + 1, e.branch ? e.key : undefined)}
  {/if}
{/snippet}

{#if body?.quiet}
  <div class="quiet">{body.quiet}</div>
{:else}
  <section>
    <SectionHead {title} open={shown} count={body?.count ?? body?.meta} ontoggle={toggle}>
      {#snippet before()}
        {#if warn}<SeverityIcon severity="warning" />{/if}
      {/snippet}
      {#snippet after()}
        {#if hint}<span class="hint" role="img" aria-label={hint} title={hint}><SeverityIcon severity="info" /></span
          >{/if}
      {/snippet}
      {#snippet aside()}
        {#if find}
          <IconButton icon="search" label={find.label} size="s" iconSize={12} onclick={find.onclick} />
        {/if}
      {/snippet}
    </SectionHead>
    {#if shown}
      {#if !load || load.state === "loading"}
        <p class="note">Loading…</p>
      {:else if load.state === "failed"}
        <p class="note err">{load.message}</p>
      {:else if body}
        {#each body.notes as note (note.text)}
          <p class="note" class:gap={note.warn}>{note.text}</p>
        {/each}
        {@render list(body.entries, 0, "")}
        {@render list(body.tail, 0, undefined)}
      {:else if children}
        {@render children(load.value)}
      {/if}
    {/if}
  </section>
{/if}

<style>
  .hint {
    display: inline-flex;
  }
  .quiet {
    display: flex;
    align-items: center;
    min-height: var(--h-ctl);
    padding: 0 10px 0 var(--inset-tree-name);
    border-top: 1px solid var(--line-soft);
    color: var(--text-3);
    font-size: var(--fs-mono);
  }
  .note {
    margin: 0;
    padding: 2px 10px 4px var(--inset-tree-name);
    line-height: 1.4;
  }
  .note.gap {
    color: var(--old-base);
  }
  .file {
    flex: none;
    width: var(--s-glyph);
    height: var(--s-icon);
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .id.code {
    font: var(--fs-mono-s) / 16px var(--font-mono);
  }
  .ctr {
    display: inline-block;
    margin-left: 6px;
    font-size: var(--fs-xs);
    line-height: 14px;
    color: var(--text-3);
  }
  .sub {
    display: block;
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .loc {
    display: none;
    flex: none;
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .ln {
    flex: none;
    font-size: var(--fs-xs);
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .fate {
    flex: none;
    font-size: var(--fs-xs);
  }
  @container (min-width: 440px) {
    .loc {
      display: block;
    }
  }
</style>

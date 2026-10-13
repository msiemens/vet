<script lang="ts">
  // What is known about the symbol on show: it need not be a changed one. Every location opens its
  // file. The signature and hover are the language server's own answers, the rest is derived
  // (SPEC §7.3, §7.4).
  import type { Calls } from "../../generated/Calls";
  import type { Reference } from "../../generated/Reference";
  import type { RemovalImpact } from "../../generated/RemovalImpact";
  import type { Side } from "../../generated/Side";
  import type { SymbolChange } from "../../generated/SymbolChange";
  import type { SymbolInfo } from "../../generated/SymbolInfo";
  import type { Usages } from "../../generated/Usages";
  import { codeBlock } from "../../lib/codeBlock";
  import { nameOf, placeLabel, shortLabel } from "../../lib/symbolLabel";
  import PathText from "../../lib/ui/PathText.svelte";
  import QualifiedName from "../../lib/ui/QualifiedName.svelte";
  import type { Shown } from "../../state/symbolLoad";
  import { symbols } from "../../state/symbols.svelte";
  import DerivedSection from "./DerivedSection.svelte";
  import SymbolGlyph from "./SymbolGlyph.svelte";
  import { containerOf, describeFate, type OpenLocation, rowTitle } from "./symbolDetail";

  let { shown, onopen }: { shown: Shown; onopen: OpenLocation } = $props();

  const id = $derived(shown.symbol.id);
  const removed = $derived(shown.change === "removed");
  const FATE = { bad: "var(--del-text)", warn: "var(--old-base)", mute: "var(--text-3)" } as const;
  const LETTER = { modified: "M", added: "A", removed: "D" } as const;
  const WORDS = { modified: "modified", added: "added", removed: "removed" } as const;

  /** What closes a row's first line: the change's letter, or the fate of a reference in words. */
  interface End {
    text: string;
    color: string;
    word?: boolean;
  }

  /** A changed symbol listed in the panel: shown in the tree and opened where it is. */
  function goTo(symbol: SymbolInfo, change: SymbolChange) {
    symbols.choose(symbol, change, true);
    const side: Side = change === "removed" ? "base" : "target";
    onopen(symbol.id.path, symbol.at.line, side);
  }
</script>

<div class="head">
  <span class="nm">
    <SymbolGlyph kind={shown.symbol.kind} />
    <span class="mono selectable" class:gone={removed}>{nameOf(id)}</span>
    {#if shown.change}<span class="chg">{WORDS[shown.change]}</span>{/if}
  </span>
  {#if containerOf(id)}
    <span class="mono q selectable"><QualifiedName name={containerOf(id)} /></span>
  {/if}
  <span class="mono q selectable"><PathText path={id.path} /></span>
</div>

<DerivedSection id="signature" title="Signature" load={symbols.signature}>
  {#snippet children(signature)}
    {#if signature.signature}
      <div class="block">
        <code class="mono selectable" use:codeBlock={signature.signature}></code>
      </div>
    {:else}
      <p class="note">{signature.unavailable?.message ?? "The language server has no text for it"}</p>
    {/if}
  {/snippet}
</DerivedSection>

{#snippet lines(label: string, elbow: boolean, below: string, suffix: string, end?: End, extra?: string)}
  <span class="top">
    {#if elbow}<span class="elbow">└</span>{/if}
    <span class="mono s"><QualifiedName name={label} /></span>
    {@render ending(end)}
  </span>
  <span class="mono under"><PathText path={below} {suffix} /></span>
  {#if extra}<span class="mono extra">{extra}</span>{/if}
{/snippet}

{#snippet ending(end?: End)}
  {#if end}<span class="end" class:mono={!end.word} style:color={end.color}>{end.text}</span>{/if}
{/snippet}

{#snippet where(reference: Reference, side: Side, end?: End, extra?: string)}
  {@const { location, within } = reference}
  {@const at = location.span.start}
  {@const label = placeLabel(within, location.path)}
  {#if location.external}
    <span class="drow two dim" title={rowTitle(within?.id ?? null, location.path, at)}>
      <span class="top">
        <span class="mono s"><QualifiedName name={label} /></span>
        {@render ending(end)}
      </span>
      <span class="mono under">outside the repository</span>
    </span>
  {:else}
    <button
      class="flat drow two"
      type="button"
      title={rowTitle(within?.id ?? null, location.path, at)}
      onclick={() => onopen(location.path, at.line, side)}
    >
      {@render lines(label, false, location.path, `:${at.line + 1}`, end, extra)}
    </button>
  {/if}
{/snippet}

{#snippet symbolRow(symbol: SymbolInfo, change: SymbolChange | undefined, depth: number, open: () => void)}
  <button
    class="flat drow two"
    class:gone={change === "removed"}
    type="button"
    style:padding-left="{26 + depth * 12}px"
    title={rowTitle(symbol.id, symbol.id.path, symbol.at)}
    onclick={open}
  >
    {@render lines(
      shortLabel(symbol),
      depth > 0,
      symbol.id.path,
      `:${symbol.at.line + 1}`,
      change && {
        text: LETTER[change],
        color: `var(--${change === "added" ? "add-text" : change === "removed" ? "del-text" : "old-base"})`,
      },
    )}
  </button>
{/snippet}

{#if removed}
  <DerivedSection
    id="remaining"
    title="Remaining references"
    load={symbols.removal}
    count={(r: RemovalImpact) => r.callSites.length}
    derived
  >
    {#snippet children(removal)}
      {#if removal.gap}<p class="note">References could not be listed: {removal.gap}</p>{/if}
      <ul>
        {#each removal.callSites as site, i (i)}
          {@const fate = describeFate(site.fate)}
          <li>
            {@render where(site.base, "base", { text: fate.label, color: FATE[fate.tone], word: true }, fate.detail)}
          </li>
        {/each}
      </ul>
    {/snippet}
  </DerivedSection>
{:else}
  <DerivedSection
    id="callers"
    title="Changed callers"
    hint="Call hierarchy cannot see calls made through interfaces or virtual methods, callbacks, reflection or generated code, so callers and callees may be missing."
    load={symbols.calls}
    count={(c: Calls) => c.callers.length}
    derived
  >
    {#snippet children(calls)}
      {#if calls.gaps.length}<p class="note">Incomplete: {calls.gaps.join("; ")}</p>{/if}
      {#each calls.callers as c, i (i)}
        {@render symbolRow(c.symbol, c.change, c.depth, () => goTo(c.symbol, c.change))}
      {:else}
        <p class="note">None</p>
      {/each}
    {/snippet}
  </DerivedSection>
  <DerivedSection
    id="callees"
    title="Changed callees"
    load={symbols.calls}
    count={(c: Calls) => c.callees.length}
    derived
  >
    {#snippet children(calls)}
      {#each calls.callees as c, i (i)}
        {@render symbolRow(c.symbol, c.change, 0, () => goTo(c.symbol, c.change))}
      {:else}
        <p class="note">None</p>
      {/each}
    {/snippet}
  </DerivedSection>
  {#if shown.change}
    <DerivedSection
      id="affected"
      title="Possibly affected"
      load={symbols.calls}
      count={(c: Calls) => c.affected.length}
      derived
      warn
    >
      {#snippet children(calls)}
        <p class="note">Unchanged direct callers; their behaviour may differ now.</p>
        {#each calls.affected as s, i (i)}
          {@render symbolRow(s, undefined, 0, () => onopen(s.id.path, s.at.line, "target"))}
        {:else}
          <p class="note">None</p>
        {/each}
      {/snippet}
    </DerivedSection>
  {/if}
  <DerivedSection id="reach" title="Reached from" load={symbols.reach} derived>
    {#snippet children(reachability)}
      {@const reach = reachability.reach}
      {#if reach.type === "reached"}
        <!-- the backend's path runs from the symbol to the entry point -->
        <ol>
          {#each [...reach.path].reverse() as step, i (i)}
            <li>
              <button
                class="flat drow two"
                type="button"
                style:padding-left="{26 + i * 10}px"
                title={rowTitle(step.id, step.id.path)}
                onclick={() => onopen(step.id.path, undefined, "target")}
              >
                {@render lines(shortLabel(step), i > 0, step.id.path, "")}
              </button>
            </li>
          {/each}
        </ol>
      {:else if reach.type === "unreachedWithinBound"}
        <p class="note">No entry point reaches it within the bounds.</p>
      {:else}
        <p class="note">Unknown: {reach.gaps.length ? reach.gaps.join("; ") : "the analysis has gaps"}</p>
      {/if}
    {/snippet}
  </DerivedSection>
  <DerivedSection id="usages" title="Usages" load={symbols.usages} count={(u: Usages) => u.references.length} derived>
    {#snippet children(usages)}
      {#if usages.gap}<p class="note">References could not be listed: {usages.gap}</p>{/if}
      <ul>
        {#each usages.references as r, i (i)}
          <li>{@render where(r, "target")}</li>
        {:else}
          {#if !usages.gap}<li class="note">None</li>{/if}
        {/each}
      </ul>
    {/snippet}
  </DerivedSection>
{/if}

<style>
  .head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--line-soft);
  }
  .nm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-ctl);
    font-weight: 500;
  }
  .q {
    display: flex;
    font-size: var(--fs-mono-s);
    color: var(--text);
  }
  .chg {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-3);
  }
  .gone {
    text-decoration: line-through;
    color: var(--text-2);
  }
  .block {
    padding: 0 10px 4px 26px;
  }
  code {
    font-size: var(--fs-mono-s);
    white-space: pre-wrap;
  }
  ul,
  ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  li.note {
    padding-left: 26px;
  }
  .drow {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    width: 100%;
    height: var(--h-row);
    padding: 0 10px 0 26px;
    box-sizing: border-box;
    white-space: nowrap;
    font-size: var(--fs-ctl);
    text-align: left;
  }
  span.drow {
    width: auto;
  }
  .drow.two {
    flex-direction: column;
    align-items: stretch;
    justify-content: center;
    gap: 0;
    height: auto;
    min-height: var(--h-row);
    padding-top: 2px;
    padding-bottom: 2px;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .under {
    display: flex;
    min-width: 0;
    font-size: 11px;
    color: var(--text-3);
  }
  .dim {
    color: var(--text-3);
  }
  .s {
    display: flex;
    min-width: 0;
    font-size: var(--fs-mono-s);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .extra {
    overflow: hidden;
    font-size: 11px;
    color: var(--text-3);
    text-overflow: ellipsis;
  }
  .elbow {
    font-size: 11px;
    color: var(--text-3);
  }
  .drow.gone .s {
    text-decoration: line-through;
    color: var(--text-2);
  }
  .end {
    flex: none;
    margin-left: auto;
    font-size: 11px;
  }
</style>

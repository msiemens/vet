<script lang="ts">
  // What is known about the selected symbol; every location opens its file. Signature
  // and hover are the language server's own answers, the rest is derived (SPEC §7.3, §7.4).
  import type { ChangedSymbol } from "../../generated/ChangedSymbol";
  import type { Impact } from "../../generated/Impact";
  import type { Reachability } from "../../generated/Reachability";
  import type { Reference } from "../../generated/Reference";
  import type { RemovalImpact } from "../../generated/RemovalImpact";
  import type { Side } from "../../generated/Side";
  import { codeBlock } from "../../lib/codeBlock";
  import { symbols } from "../../state/symbols.svelte";
  import DerivedSection from "./DerivedSection.svelte";
  import { containerOf, describeFate, nameOf, type OpenLocation, place, rowLabel } from "./symbolList";

  let { symbol, onopen }: { symbol: ChangedSymbol; onopen: OpenLocation } = $props();

  const id = $derived(symbol.symbol.id);
  const signature = $derived(symbols.signature);
  const FATE = { bad: "var(--del-text)", warn: "var(--old-base)", mute: "var(--text-3)" } as const;
</script>

<div class="head">
  <span class="mono name selectable">{nameOf(id)}</span>
  <span class="mono qualified selectable">{[id.path, containerOf(id)].filter(Boolean).join(" · ")}</span>
  <span class="change">{symbol.change}</span>
</div>

<section class="plain">
  <div class="sh">Signature</div>
  {#if !signature || signature.state === "loading"}
    <p class="note">Loading…</p>
  {:else if signature.state === "failed"}
    <p class="note err">{signature.message}</p>
  {:else if signature.value.signature}
    <div class="block">
      <code class="mono selectable" use:codeBlock={signature.value.signature}></code>
    </div>
  {:else}
    <p class="note">{signature.value.unavailable?.message ?? "The language server has no text for it"}</p>
  {/if}
</section>

{#snippet where(reference: Reference, side: Side)}
  {@const { location, within } = reference}
  {@const at = location.span.start}
  {@const label = rowLabel(within, location.path)}
  {#if location.external}
    <span class="tw dim" title={place(location.path, at)}
      ><span class="mono s">{label.name}</span><span class="at">outside the repository</span></span
    >
  {:else}
    <button
      class="flat tw"
      type="button"
      title={place(location.path, at)}
      onclick={() => onopen(location.path, at.line, side)}
    >
      <span class="mono s">{label.name}</span><span class="mono at">{label.context}</span>
    </button>
  {/if}
{/snippet}

{#if symbol.change === "removed"}
  <DerivedSection
    title="Remaining references"
    load={symbols.removal}
    count={(r: RemovalImpact) => r.callSites.length}
    inputs={(r: RemovalImpact) => r.inputs}
  >
    {#snippet children(removal)}
      {#if removal.gap}<p class="note">References could not be listed: {removal.gap}</p>{/if}
      <ul>
        {#each removal.callSites as site, i (i)}
          {@const fate = describeFate(site.fate)}
          <li>
            {@render where(site.base, "base")}
            <span class="verdict" style:color={FATE[fate.tone]}>{fate.label}</span>
            {#if fate.detail}<span class="mono at">{fate.detail}</span>{/if}
          </li>
        {/each}
      </ul>
    {/snippet}
  </DerivedSection>
{:else}
  <DerivedSection
    title="Callers"
    load={symbols.impact}
    count={(i: Impact) => i.callers.length}
    inputs={(i: Impact) => i.inputs}
  >
    {#snippet children(impact)}
      {#if impact.gaps.length}<p class="note">Incomplete: {impact.gaps.join("; ")}</p>{/if}
      <ul>
        {#each impact.callers as c, i (i)}
          {@const label = rowLabel(c.id, c.id.path)}
          <li>
            <button
              class="flat tw"
              type="button"
              title={place(c.id.path, c.at)}
              onclick={() => onopen(c.id.path, c.at.line, "target")}
            >
              <span class="mono s">{label.name}</span><span class="mono at">{label.context}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/snippet}
  </DerivedSection>
  <DerivedSection
    title="References"
    load={symbols.impact}
    count={(i: Impact) => i.references.length}
    inputs={(i: Impact) => i.inputs}
  >
    {#snippet children(impact)}
      <ul>
        {#each impact.references as r, i (i)}
          <li>{@render where(r, "target")}</li>
        {/each}
      </ul>
    {/snippet}
  </DerivedSection>
  <DerivedSection title="Reached from" load={symbols.reach} inputs={(r: Reachability) => r.inputs}>
    {#snippet children(reachability)}
      {@const reach = reachability.reach}
      {#if reach.type === "reached"}
        <!-- the backend's path runs from the changed symbol to the entry point -->
        <ol>
          {#each [...reach.path].reverse() as step, i (i)}
            {@const label = rowLabel(step, step.path)}
            <li>
              <button
                class="flat tw"
                type="button"
                style:padding-left="{26 + i * 10}px"
                title={step.path}
                onclick={() => onopen(step.path, undefined, "target")}
              >
                {#if i > 0}<span class="at">└</span>{/if}<span class="mono s">{label.name}</span><span class="mono at"
                  >{label.context}</span
                >
              </button>
            </li>
          {/each}
        </ol>
      {:else if reach.type === "unreachedWithinBound"}
        <p class="note">No entry point reaches it within the bounds (see the inputs).</p>
      {:else}
        <p class="note">Unknown: {reach.gaps.length ? reach.gaps.join("; ") : "the analysis has gaps"}</p>
      {/if}
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
  .name {
    font-size: var(--fs-ctl);
    font-weight: 500;
  }
  .qualified {
    font-size: var(--fs-mono-s);
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .change {
    font-size: 11px;
    color: var(--text-3);
  }
  section.plain {
    border-bottom: 1px solid var(--line-soft);
    padding-bottom: 6px;
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
  .tw {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: var(--h-row);
    padding: 0 10px 0 26px;
    white-space: nowrap;
    font-size: var(--fs-ctl);
    text-align: left;
  }
  button.tw {
    width: 100%;
  }
  .dim {
    color: var(--text-3);
  }
  .s {
    font-size: var(--fs-mono-s);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .at {
    font-size: 11px;
    color: var(--text-3);
  }
  .verdict {
    font-size: 12px;
    white-space: nowrap;
    padding-right: 10px;
  }
</style>

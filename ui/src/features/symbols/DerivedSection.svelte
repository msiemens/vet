<script lang="ts" generics="T">
  // One section of the detail view: a titled answer that is loading, failed or shown. A
  // derived answer is labelled so and lists what it was derived from on request (I6).
  import type { Snippet } from "svelte";
  import type { DerivedInputs } from "../../generated/DerivedInputs";
  import Icon from "../../lib/ui/Icon.svelte";
  import type { Load } from "../../state/symbols.svelte";

  let {
    title,
    load,
    count,
    inputs,
    children,
  }: {
    title: string;
    load?: Load<T>;
    /** The number of rows once loaded. */
    count?: (value: T) => number;
    /** Present for a derived answer. */
    inputs?: (value: T) => DerivedInputs;
    children: Snippet<[T]>;
  } = $props();

  let showInputs = $state(false);
</script>

<section>
  <div class="sh">
    <Icon name="open" size={10} />{title}
    {#if load?.state === "done" && count}<span class="mono n">{count(load.value)}</span>{/if}
    <span class="grow"></span>
    {#if inputs}
      <span class="derived" title="Computed from the primitive answers below, not reported by a language server">
        <Icon name="derived" size={12} />Derived
      </span>
    {/if}
  </div>
  {#if !load || load.state === "loading"}
    <p class="note">Loading…</p>
  {:else if load.state === "failed"}
    <p class="note err">{load.message}</p>
  {:else}
    {@render children(load.value)}
    {#if inputs}
      {@const used = inputs(load.value)}
      <button class="flat disclose" type="button" aria-expanded={showInputs} onclick={() => (showInputs = !showInputs)}>
        <Icon name={showInputs ? "open" : "closed"} size={9} />{showInputs ? "Hide inputs" : "Show inputs"}
      </button>
      {#if showInputs}
        <dl class="inputs mono selectable">
          <dt>analysis</dt>
          <dd>
            {used.analysis}{#if used.bounds}
              (depth ≤ {used.bounds.maxDepth}, nodes ≤ {used.bounds.maxNodes}){/if}
          </dd>
          {#each [["queries", used.primitives], ["hunks", used.hunks], ["config", used.config]] as const as [label, items] (label)}
            {#if items.length}
              <dt>{label}</dt>
              {#each items as item, i (i)}<dd>{item}</dd>{/each}
            {/if}
          {/each}
        </dl>
      {/if}
    {/if}
  {/if}
</section>

<style>
  section {
    border-bottom: 1px solid var(--line-soft);
    padding-bottom: 6px;
  }
  .derived {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-weight: 400;
    font-size: 11px;
    color: var(--text-3);
  }
  .disclose {
    display: flex;
    align-items: center;
    gap: 5px;
    height: var(--h-row);
    margin-left: 16px;
    padding: 0 6px;
    font-size: 11px;
    color: var(--text-3);
  }
  .inputs {
    margin: 0;
    padding: 2px 10px 4px 26px;
    font-size: var(--fs-mono-s);
    color: var(--text-2);
  }
  dt {
    margin-top: 4px;
    color: var(--text-3);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>

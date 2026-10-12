<script lang="ts" generics="T">
  // One section of the detail view: a titled answer that is loading, failed or shown. A
  // derived answer is labelled so (I6).
  import type { Snippet } from "svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import type { Load } from "../../state/symbols.svelte";

  let {
    title,
    load,
    count,
    derived = false,
    children,
  }: {
    title: string;
    load?: Load<T>;
    /** The number of rows once loaded. */
    count?: (value: T) => number;
    /** The answer is computed from the primitive ones. */
    derived?: boolean;
    children: Snippet<[T]>;
  } = $props();
</script>

<section>
  <div class="sh">
    <Icon name="open" size={10} />{title}
    {#if load?.state === "done" && count}<span class="mono n">{count(load.value)}</span>{/if}
    <span class="grow"></span>
    {#if derived}
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
</style>

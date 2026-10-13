<script lang="ts" generics="T">
  // One section of the details: a titled answer that is loading, failed or shown, which opens and
  // shuts and is remembered so. A derived answer is labelled so (I6).
  import type { Snippet } from "svelte";
  import { readStored, writeStored } from "../../lib/storage";
  import Icon from "../../lib/ui/Icon.svelte";
  import type { Load } from "../../state/symbols.svelte";

  let {
    id,
    title,
    load,
    count,
    derived = false,
    warn = false,
    children,
  }: {
    /** Names the section where its state is kept. */
    id: string;
    title: string;
    load?: Load<T>;
    /** The number of rows once loaded. */
    count?: (value: T) => number;
    /** The answer is computed from the primitive ones. */
    derived?: boolean;
    /** The section says something to be careful about. */
    warn?: boolean;
    children: Snippet<[T]>;
  } = $props();

  const KEY = "vet.symbols.section.";
  // The section keeps the state it was first shown with, whatever its title becomes.
  // svelte-ignore state_referenced_locally
  let open = $state(readStored(KEY + id) !== "shut");

  function toggle() {
    open = !open;
    writeStored(KEY + id, open ? undefined : "shut");
  }
</script>

<section>
  <button class="flat sh head" type="button" aria-expanded={open} onclick={toggle}>
    {#if warn}
      <span class="lead warn"><Icon name="warning" size={12} /></span>
    {:else}
      <span class="lead"><Icon name={open ? "open" : "closed"} size={10} /></span>
    {/if}
    {title}
    {#if load?.state === "done" && count}<span class="mono n">{count(load.value)}</span>{/if}
    <span class="grow"></span>
    {#if derived}
      <span class="derived" title="Computed from the primitive answers, not reported by a language server">
        <Icon name="derived" size={12} />Derived
      </span>
    {/if}
  </button>
  {#if open}
    {#if !load || load.state === "loading"}
      <p class="note">Loading…</p>
    {:else if load.state === "failed"}
      <p class="note err">{load.message}</p>
    {:else}
      {@render children(load.value)}
    {/if}
  {/if}
</section>

<style>
  section {
    border-bottom: 1px solid var(--line-soft);
    padding-bottom: 6px;
  }
  .head {
    width: 100%;
    text-align: left;
  }
  .lead {
    display: inline-flex;
  }
  .warn {
    color: var(--old-base);
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

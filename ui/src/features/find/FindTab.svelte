<script lang="ts">
  // A kept result set in the strip of the Find tool: what it is, how many places it has, whether the
  // next result replaces it (an italic title) or it is pinned.
  import Icon from "../../lib/ui/Icon.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import Tab from "../../lib/ui/Tab.svelte";
  import type { FindSet } from "../../state/findSets.svelte";
  import { tabTitle } from "./findWords";

  let {
    set,
    selected,
    count,
    stale,
    onselect,
    onpin,
    onclose,
  }: {
    set: FindSet;
    selected: boolean;
    /** The places the set shows now; none while it runs for the first time. */
    count?: number;
    /** The code changed since the set ran. */
    stale: boolean;
    onselect: () => void;
    onpin: () => void;
    onclose: () => void;
  } = $props();

  const commit = $derived(set.spec.type === "search" ? set.spec.scope.commit : undefined);
</script>

<Tab {selected} title={tabTitle(set.spec.type, set.title, set.pinned)} {onselect} {onclose}>
  <span class="kind">
    {#if set.phase === "running" && !set.hasRun}<Spinner />{:else}<Icon
        name={set.spec.type === "usages" ? "usages" : "search"}
        size={13}
      />{/if}
  </span>
  <span class="name mono" class:loose={!set.pinned}>{set.title}</span>
  {#if count !== undefined}<span class="n mono">{count}</span>{/if}
  {#if stale}<span class="stale" title="The code changed since this ran"></span>{/if}
  {#if commit}<span class="at" title="Searches the text at commit {commit.shortId}"
      ><Icon name="commit" size={11} /></span
    >{/if}
  {#snippet actions()}
    <button
      class="flat pin"
      class:on={set.pinned}
      type="button"
      aria-label={set.pinned ? "Unpin tab" : "Pin tab"}
      aria-pressed={set.pinned}
      title={set.pinned ? "Unpin tab" : "Pin tab: keep it when the next result comes"}
      tabindex="-1"
      onclick={onpin}><Icon name="pin" size={12} /></button
    >
  {/snippet}
</Tab>

<style>
  .kind {
    display: inline-flex;
    flex: none;
    color: var(--text-3);
  }
  .name {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--fs-mono-s);
  }
  .name.loose {
    font-style: italic;
  }
  .stale {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--old-base);
  }
  .at {
    display: inline-flex;
    flex: none;
    color: var(--text-3);
  }
  .pin {
    width: var(--s-icon-l);
    height: var(--s-icon-l);
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    color: var(--text-3);
  }
  .pin.on {
    color: var(--accent);
  }
  .pin.on :global(svg) {
    fill: currentColor;
  }
</style>

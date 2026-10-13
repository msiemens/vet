<script lang="ts">
  // A kept result set in the strip of the Find tool: what it is, how many places it has, whether the
  // next result replaces it (an italic title) or it is pinned.
  import Icon from "../../lib/ui/Icon.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
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

<div
  class="tab"
  class:selected
  class:loose={!set.pinned}
  role="presentation"
  onmousedown={(e) => {
    if (e.button === 0 && !(e.target instanceof Element && e.target.closest(".act"))) onselect();
    if (e.button === 1) e.preventDefault();
  }}
  onauxclick={(e) => e.button === 1 && onclose()}
>
  <button
    class="label"
    type="button"
    role="tab"
    aria-selected={selected}
    title={tabTitle(set.spec.type, set.title, set.pinned)}
    onclick={onselect}
  >
    <span class="kind">
      {#if set.phase === "running" && !set.hasRun}<Spinner />{:else}<Icon
          name={set.spec.type === "usages" ? "usages" : "search"}
          size={13}
        />{/if}
    </span>
    <span class="name mono">{set.title}</span>
    {#if count !== undefined}<span class="n mono">{count}</span>{/if}
    {#if stale}<span class="stale" title="The code changed since this ran"></span>{/if}
    {#if commit}<span class="at" title="Searches the text at commit {commit.shortId}"
        ><Icon name="commit" size={11} /></span
      >{/if}
  </button>
  <button
    class="flat act pin"
    class:on={set.pinned}
    type="button"
    aria-label={set.pinned ? "Unpin tab" : "Pin tab"}
    aria-pressed={set.pinned}
    title={set.pinned ? "Unpin tab" : "Pin tab: keep it when the next result comes"}
    tabindex="-1"
    onclick={onpin}><Icon name="pin" size={12} /></button
  >
  <button class="flat act close" type="button" aria-label="Close tab" title="Close tab" tabindex="-1" onclick={onclose}>
    <Icon name="close" size={10} stroke={2} />
  </button>
</div>

<style>
  .tab {
    display: inline-flex;
    align-items: center;
    flex: none;
    max-width: 230px;
    height: 100%;
    box-sizing: border-box;
    padding-right: 4px;
    border-bottom: 2px solid transparent;
    color: var(--text-2);
    font-size: 12px;
  }
  .tab:not(.selected):hover {
    background: var(--hover);
  }
  .tab.selected {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    height: 100%;
    padding: 0 4px 0 10px;
    border: 0;
    background: transparent;
    color: inherit;
    font-size: inherit;
    white-space: nowrap;
  }
  .kind {
    display: inline-flex;
    flex: none;
    color: var(--text-3);
  }
  .selected .kind {
    color: var(--accent);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--fs-mono-s);
  }
  .loose .name {
    font-style: italic;
  }
  .n {
    flex: none;
    color: var(--text-3);
    font-size: 11px;
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
  .act {
    width: 17px;
    height: 17px;
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    color: var(--text-3);
    opacity: 0;
  }
  .tab:hover .act,
  .tab.selected .act,
  .pin.on {
    opacity: 1;
  }
  .pin.on {
    color: var(--accent);
  }
  .pin.on :global(svg) {
    fill: currentColor;
  }
</style>

<script lang="ts">
  import type { FileChange } from "../../generated/FileChange";
  import { basename, dirname } from "../../lib/paths";
  import { ListNav } from "../../lib/ui/listNav";
  import Meter from "../../lib/ui/Meter.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";

  let {
    paths,
    fileOf,
    onopen,
  }: {
    paths: readonly string[];
    /** A file as it is now, with its review state. */
    fileOf: (path: string) => FileChange | undefined;
    /** Opens the review at the file: a double click or Enter. */
    onopen: (path: string) => void;
  } = $props();

  let selected = $state<string>();
  let highlighted = $state(-1);
  let rows = $state<VirtualRows>();
  let list = $state<HTMLElement>();
  const nav = new ListNav<string>({
    element: () => list,
    items: () => paths,
    single: (path) => (selected = path),
    double: (path) => onopen(path),
    highlight: { get: () => highlighted, set: (i) => (highlighted = i) },
    reveal: (i) => void rows?.reveal(i),
  });
  $effect(() => () => nav.cancel());

  function onkeydown(e: KeyboardEvent) {
    if (!nav.keydown(e)) return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

{#snippet row(index: number)}
  {@const path = paths[index]}
  {@const file = fileOf(path)}
  <button
    class="flat file"
    class:on={path === selected}
    class:cur={index === highlighted}
    type="button"
    tabindex="-1"
    onclick={(e) => nav.click(e, index)}
  >
    <span class="name">{basename(path)}</span>
    <span class="mono dir">{dirname(path)}</span>
    {#if file}
      <span class="mono delta"
        ><span class="add">+{file.delta.added}</span> <span class="del">−{file.delta.removed}</span></span
      >
      <Meter
        width={56}
        segments={[
          { value: file.completion.seenPercent, color: "var(--seen)" },
          { value: file.completion.oldBasePercent, color: "var(--old-base-stripes)" },
        ]}
      />
    {/if}
  </button>
{/snippet}

{#if paths.length}
  <!-- The list takes the keys of its rows, which are not in the tab order. -->
  <div class="list" role="presentation" bind:this={list} {onkeydown} {...nav.attributes}>
    <VirtualRows bind:this={rows} count={paths.length} key={(i) => paths[i]} {row} />
  </div>
{:else}
  <p class="empty">No changed files</p>
{/if}

<style>
  .list {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
  }
  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: var(--h-row);
    padding: 0 16px;
    box-sizing: border-box;
    border-radius: 0;
    text-align: left;
    font-size: var(--fs-ctl);
  }
  .file.on {
    background: var(--toggle);
  }
  /* The highlighted row, which Space and Enter act on, while the list has the focus. */
  .list:focus-within .file.cur {
    background: var(--select);
  }
  .name {
    flex: none;
  }
  .dir {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .delta {
    font-size: var(--fs-mono-s);
  }
  .add {
    color: var(--add-text);
  }
  .del {
    color: var(--del-text);
  }
  .empty {
    margin: 12px 16px;
    color: var(--text-3);
  }
</style>

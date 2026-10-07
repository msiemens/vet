<script lang="ts">
  import type { FileChange } from "../../generated/FileChange";
  import { basename, dirname } from "../../lib/paths";
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
    /** Opens the review at the file. */
    onopen: (path: string) => void;
  } = $props();

  let selected = $state<string>();
</script>

{#snippet row(index: number)}
  {@const path = paths[index]}
  {@const file = fileOf(path)}
  <button
    class="flat file"
    class:on={path === selected}
    type="button"
    onclick={() => (selected = path)}
    ondblclick={() => onopen(path)}
    onkeydown={(e) => {
      if (e.key === "Enter") onopen(path);
    }}
  >
    <span class="name">{basename(path)}</span>
    <span class="mono dir">{dirname(path)}</span>
    {#if file}
      <span class="mono delta"
        ><span class="add">+{file.delta.added}</span> <span class="del">−{file.delta.removed}</span></span
      >
      <Meter width={56} segments={[{ value: file.completion.seenPercent, color: "var(--seen)" }]} />
    {/if}
  </button>
{/snippet}

{#if paths.length}
  <VirtualRows count={paths.length} key={(i) => paths[i]} {row} />
{:else}
  <p class="empty">No changed files</p>
{/if}

<style>
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

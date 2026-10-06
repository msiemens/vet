<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import type { FileChange } from "../../generated/FileChange";
  import { type FileRow, buildRows } from "./tree";

  let {
    files,
    selected,
    excluded,
    onselect,
  }: {
    files: FileChange[];
    selected?: string;
    /** Files hidden by the review filter, collapsed into one group. */
    excluded?: { label: string; count: number };
    onselect: (row: FileRow) => void;
  } = $props();

  let collapsed = $state(new Set<string>());
  const rows = $derived(buildRows(files, collapsed));

  const STATUS = { M: "var(--old)", A: "var(--addt)", D: "var(--delt)", R: "var(--tx2)" } as const;

  function toggle(path: string) {
    const next = new Set(collapsed);
    if (!next.delete(path)) next.add(path);
    collapsed = next;
  }
</script>

<nav aria-label="Changed files">
  <div class="sh"><span>Files</span><span class="grow"></span><span class="mono count">{files.length}</span></div>
  <div class="rows">
    {#each rows as row (row.path + row.kind)}
      {#if row.kind === "dir"}
        <button class="tw dir" type="button" aria-expanded={!collapsed.has(row.path)} onclick={() => toggle(row.path)}>
          <Icon name={collapsed.has(row.path) ? "closed" : "open"} size={10} />{row.path}
        </button>
      {:else}
        <button class="tw file" class:sel={row.path === selected} class:nested={row.path.includes("/")} type="button" onclick={() => onselect(row)}>
          {#if row.build}<span class="build" title="Build file"><Icon name="build" size={12} stroke={2} /></span>{/if}
          <span class="name">{row.name}</span>
          {#if row.executable}<span class="mono x" title="File mode changed">mode</span>{/if}
          <Meter width={22} segments={[{ value: row.seenPct, color: "var(--seen)" }, { value: row.oldPct, color: "var(--old)" }]} />
          <span class="mono status" style:color={STATUS[row.status]}>{row.status}</span>
        </button>
      {/if}
    {/each}
    {#if excluded && excluded.count > 0}
      <div class="tw excluded" title="Excluded by the review filter">
        <Icon name="closed" size={10} /><Icon name="filter" size={11} stroke={2} />{excluded.label}<span class="mono x">{excluded.count}</span>
      </div>
    {/if}
  </div>
</nav>

<style>
  nav { flex: 0 1 var(--w-tree); min-width: 230px; background: var(--panel); border-right: 1px solid var(--ln); display: flex; flex-direction: column; min-height: 0; }
  .sh { display: flex; align-items: center; gap: 6px; height: var(--h-ctl); padding: 0 10px; font-size: 12px; font-weight: 600; color: var(--tx2); border-bottom: 1px solid var(--ln2); flex: none; }
  .grow { flex: 1; }
  .count { font-weight: 400; color: var(--tx3); }
  .rows { padding: 4px 0; overflow: hidden auto; min-height: 0; }
  .tw { display: flex; align-items: center; gap: 6px; height: var(--h-row); width: 100%; padding: 0 10px 0 8px; border: 0; background: transparent; text-align: left; white-space: nowrap; font-size: var(--fs-ctl); cursor: pointer; box-sizing: border-box; }
  .dir { color: var(--tx2); overflow: hidden; text-overflow: ellipsis; }
  .file.nested { padding-left: 26px; }
  .file:hover { background: var(--hov); }
  .file.sel { background: var(--sel); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .x { font-size: 11px; color: var(--tx2); }
  .status { width: 10px; font-size: 11px; }
  .build { display: inline-flex; color: var(--warn); }
  .excluded { color: var(--tx3); cursor: default; }
</style>

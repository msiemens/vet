<script module lang="ts">
  export interface RunTarget {
    id: string;
    name: string;
    /** Where the manifest scan found it (Cargo bin, package script, pyproject script). */
    source: string;
  }
</script>

<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import SplitButton from "../../lib/ui/SplitButton.svelte";
  import { mockTargets } from "./mock";

  let {
    targets = mockTargets,
    buildFilesChanged = true,
    onrun,
  }: {
    targets?: RunTarget[];
    /** Executor-relevant files changed since the last approved run (SPEC §7.5). */
    buildFilesChanged?: boolean;
    onrun?: (id: string) => void;
  } = $props();

  let selected = $state<string>();
  const current = $derived(targets.find((t) => t.id === selected) ?? targets[0]);
</script>

<SplitButton
  label={buildFilesChanged ? "Run, build files changed" : "Run"}
  items={targets.map((t) => ({ id: t.id, label: t.name, detail: t.source }))}
  onmain={() => current && onrun?.(current.id)}
  onselect={(id) => ((selected = id), onrun?.(id))}
>
  {#snippet main()}
    <span class="play"><Icon name="play" size={12} /></span>{current?.name ?? "Run"}{#if buildFilesChanged}<span class="dot"></span>{/if}
  {/snippet}
  {#snippet menuFooter()}
    <button type="button" class="edit">Edit targets…</button>
  {/snippet}
</SplitButton>

<style>
  .play { display: inline-flex; color: var(--ok); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--warn); }
  .edit { width: 100%; height: var(--h-row); border: 0; background: transparent; text-align: left; padding: 0 12px; font-size: var(--fs-ctl); cursor: pointer; }
  .edit:hover { background: var(--hov); }
</style>

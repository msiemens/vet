<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import { MODES, type Mode } from "../../lib/viewMode";

  /** A changed file's tab shows its text or its diff. */
  type FileView = "file" | "diff";

  let {
    mode,
    onmode,
    onstep,
    diff,
    shown,
    onshow,
  }: {
    mode: Mode;
    onmode: (mode: Mode) => void;
    /** Moves the current hunk by `delta` in the focused tab. */
    onstep: (delta: number) => void;
    /** The focused tab shows a diff: it has a view mode and hunks. */
    diff: boolean;
    /** What the focused tab of a changed file shows, which it can switch; absent in any other tab. */
    shown?: FileView;
    onshow: (view: FileView) => void;
  } = $props();

  const VIEWS: { id: FileView; label: string }[] = [
    { id: "file", label: "File" },
    { id: "diff", label: "Diff" },
  ];
</script>

<div class="controls">
  {#if shown}
    <Segmented small label="Show" value={shown} onchange={onshow} options={VIEWS} />
  {/if}
  {#if diff}
    <Segmented small label="View" value={mode} onchange={onmode} options={MODES} />
    <div class="ctl-group" role="group" aria-label="Hunk navigation">
      <IconButton
        icon="up"
        label={commands.tooltip("review.previousHunk")}
        size={22}
        iconSize={14}
        onclick={() => onstep(-1)}
      />
      <IconButton
        icon="down"
        label={commands.tooltip("review.nextHunk")}
        size={22}
        iconSize={14}
        onclick={() => onstep(1)}
      />
    </div>
  {/if}
</div>

<style>
  .controls {
    display: flex;
    gap: 6px;
    padding: 0 6px;
    flex: none;
  }
</style>

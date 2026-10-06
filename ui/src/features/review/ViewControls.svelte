<script lang="ts">
  import type { Mode } from "./editor/mount";

  let {
    mode,
    onmode,
    onstep,
  }: {
    mode: Mode;
    onmode: (mode: Mode) => void;
    /** Moves the current hunk by `delta` in the focused tab. */
    onstep: (delta: number) => void;
  } = $props();
</script>

<div class="controls">
  <div class="ctl-group" role="group" aria-label="View">
    <button class="ctl" type="button" aria-pressed={mode === "unified"} onclick={() => onmode("unified")}
      >Unified</button
    >
    <button class="ctl" type="button" aria-pressed={mode === "split"} onclick={() => onmode("split")}>
      Side by side
    </button>
  </div>
  <div class="ctl-group" role="group" aria-label="Hunk navigation">
    <button class="ctl" type="button" title="Ctrl-Alt-Up" onclick={() => onstep(-1)}>Prev hunk</button>
    <button class="ctl" type="button" title="Ctrl-Alt-Down" onclick={() => onstep(1)}>Next hunk</button>
  </div>
</div>

<style>
  .controls {
    display: flex;
    gap: 6px;
    padding: 0 6px;
    flex: none;
  }
  .ctl {
    height: 22px;
    font-size: 12px;
  }
</style>

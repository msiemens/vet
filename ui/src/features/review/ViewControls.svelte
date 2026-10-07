<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import { MODES, type Mode } from "../../lib/viewMode";

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
</div>

<style>
  .controls {
    display: flex;
    gap: 6px;
    padding: 0 6px;
    flex: none;
  }
</style>

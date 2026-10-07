<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import { MODES, type Mode } from "../../lib/viewMode";

  let {
    mode,
    onmode,
    onstep,
    wrapped,
    onwrap,
    diff,
  }: {
    mode: Mode;
    onmode: (mode: Mode) => void;
    /** Moves the current hunk by `delta` in the focused tab. */
    onstep: (delta: number) => void;
    /** The focused tab wraps long lines. */
    wrapped: boolean;
    onwrap: () => void;
    /** The focused tab shows a diff: it has a view mode and hunks. */
    diff: boolean;
  } = $props();
</script>

<div class="controls">
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
  <IconButton
    icon="wrap"
    label={commands.tooltip("review.toggleWrap")}
    size={22}
    iconSize={14}
    active={wrapped}
    onclick={onwrap}
  />
</div>

<style>
  .controls {
    display: flex;
    gap: 6px;
    padding: 0 6px;
    flex: none;
  }
</style>

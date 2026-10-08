<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import { FILE_VIEW, MODES, type Mode } from "../../lib/viewMode";

  /** What a changed file's tab shows: its diff in the view mode, or its text. */
  type FileView = "diff" | "file";

  let {
    mode,
    onmode,
    onstep,
    stepping,
    file,
    onfile,
  }: {
    mode: Mode;
    onmode: (mode: Mode) => void;
    /** Moves the current hunk by `delta` in the focused tab. */
    onstep: (delta: number) => void;
    /** The focused tab has hunks to step through, and a view mode. */
    stepping: boolean;
    /** What the focused tab of a changed file shows, which it can switch; absent in any other tab. */
    file?: FileView;
    /** Switches the focused tab between the file's diff and its text. */
    onfile: () => void;
  } = $props();

  const options = $derived(file ? [...MODES, FILE_VIEW] : MODES);
  const value = $derived(file === "file" ? FILE_VIEW.id : mode);

  /** A view mode of the diff, from the text, takes the tab back to the diff. */
  function choose(id: Mode | typeof FILE_VIEW.id) {
    if (id === FILE_VIEW.id) {
      if (file === "diff") onfile();
      return;
    }
    if (id !== mode) onmode(id);
    if (file === "file") onfile();
  }
</script>

<div class="controls">
  {#if stepping}
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
  {#if stepping || file}
    <Segmented small label="View" {value} onchange={choose} {options} />
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

<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import { FILE_VIEW, MODES, type Mode } from "../../lib/viewMode";

  /** What a changed file's tab shows: its diff in the view mode, or its text. */
  type FileView = "diff" | "file";

  let {
    mode,
    stepping,
    file,
  }: {
    mode: Mode;
    /** The focused tab has hunks to step through, and a view mode. */
    stepping: boolean;
    /** What the focused tab of a changed file shows, which it can switch; absent in any other tab. */
    file?: FileView;
  } = $props();

  const options = $derived([
    ...MODES.map((m) => ({
      ...m,
      title: commands.disabledReason("view.splitDiff"),
      disabled: !commands.isEnabled("view.splitDiff"),
    })),
    ...(file
      ? [{ ...FILE_VIEW, title: commands.hint("view.fileText"), disabled: !commands.isEnabled("view.fileText") }]
      : []),
  ]);
  const value = $derived(file === "file" ? FILE_VIEW.id : mode);

  /** A view mode of the diff, from the text, takes the tab back to the diff. */
  function choose(id: Mode | typeof FILE_VIEW.id) {
    if (id === FILE_VIEW.id) {
      if (file === "diff") commands.run("view.fileText");
      return;
    }
    if (id !== mode) commands.run("view.splitDiff");
    if (file === "file") commands.run("view.fileText");
  }
</script>

<div class="controls">
  {#if stepping}
    <div class="ctl-group" role="group" aria-label="Hunk navigation">
      {#each [{ id: "review.previousHunk", icon: "up" }, { id: "review.nextHunk", icon: "down" }] as const as step (step.id)}
        <IconButton
          icon={step.icon}
          label={commands.label(step.id)}
          title={commands.hint(step.id)}
          disabled={!commands.isEnabled(step.id)}
          size={22}
          iconSize={14}
          onclick={() => commands.run(step.id)}
        />
      {/each}
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

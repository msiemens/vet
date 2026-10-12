<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import { FILE_VIEW, MODES, type Mode, type ViewMode, modeCommand } from "../../lib/viewMode";

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

  const options = $derived(
    [...MODES, ...(file ? [FILE_VIEW] : [])].map((m) => ({
      ...m,
      title: commands.hint(modeCommand(m.id)),
      disabled: !commands.isEnabled(modeCommand(m.id)),
    })),
  );
  const value = $derived(file === "file" ? FILE_VIEW.id : mode);

  const choose = (id: ViewMode) => commands.run(modeCommand(id));
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

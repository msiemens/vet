<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { commands } from "../../state/commands";
  import { FILE_VIEW, MODES, type ViewMode, modeCommand } from "../../lib/viewMode";

  let {
    mode,
    stepping,
    file,
  }: {
    /** What the focused tab shows, which the control switches; none for a log. */
    mode?: ViewMode;
    /** The focused tab has hunks to step through. */
    stepping: boolean;
    /** The focused tab is a file's, which can show its text. */
    file: boolean;
  } = $props();

  const options = $derived(
    [...MODES, ...(file ? [FILE_VIEW] : [])].map((m) => ({
      ...m,
      title: commands.hint(modeCommand(m.id)),
      disabled: !commands.isEnabled(modeCommand(m.id)),
    })),
  );

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
  {#if mode !== undefined}
    <Segmented small label="View" value={mode} onchange={choose} {options} />
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

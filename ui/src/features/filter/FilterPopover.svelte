<script lang="ts">
  import type { ReviewFilter } from "../../generated/ReviewFilter";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Popover from "../../lib/ui/Popover.svelte";

  let {
    filter,
    excluded,
    onsave,
  }: {
    /** The session's filter, as the manifest carries it. */
    filter: ReviewFilter;
    /** Changes the filter currently excludes; the backend computes them. */
    excluded: { lines: number; items: number };
    /** Saves a changed filter; resolves with the backend's message if it refused it. */
    onsave: (filter: ReviewFilter) => Promise<string | undefined>;
  } = $props();

  let open = $state(false);
  let error = $state<string>();
  const lines = (text: string) =>
    text
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean);

  /** A refused edit stays in its field with the message below; an accepted one comes back as the new manifest's filter. */
  async function save(change: Partial<ReviewFilter>) {
    error = await onsave({ ...filter, ...change });
  }
</script>

<Popover bind:open>
  {#snippet trigger(toggle)}
    <IconButton icon="filter" label="Review filter" active={open} onclick={toggle} />
  {/snippet}
  <form onsubmit={(e) => e.preventDefault()}>
    <label>
      <span>Review roots</span>
      <textarea
        class="mono"
        rows="2"
        value={filter.roots.join("\n")}
        onchange={(e) => save({ roots: lines(e.currentTarget.value) })}></textarea>
    </label>
    <label class="check"
      ><input
        type="checkbox"
        checked={filter.hideBinary}
        onchange={(e) => save({ hideBinary: e.currentTarget.checked })}
      />Hide binary files</label
    >
    <label>
      <span>Include globs</span>
      <textarea
        class="mono"
        rows="2"
        value={filter.include.join("\n")}
        onchange={(e) => save({ include: lines(e.currentTarget.value) })}></textarea>
    </label>
    <label>
      <span>Exclude globs</span>
      <textarea
        class="mono"
        rows="3"
        value={filter.exclude.join("\n")}
        onchange={(e) => save({ exclude: lines(e.currentTarget.value) })}></textarea>
    </label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <p class="count">
      <span class="mono">{excluded.lines}</span> changed lines and <span class="mono">{excluded.items}</span> other changes
      excluded
    </p>
  </form>
</Popover>

<style>
  form {
    width: 320px;
    padding: 12px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: var(--fs-ctl);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-2);
  }
  .check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--text);
  }
  textarea {
    resize: vertical;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-editor);
    color: var(--text);
    font-size: var(--fs-mono);
    padding: 4px 8px;
  }
  .error {
    margin: 0;
    color: var(--del-text);
  }
  .count {
    margin: 0;
    padding-top: 8px;
    border-top: 1px solid var(--line-soft);
    color: var(--text-2);
  }
</style>

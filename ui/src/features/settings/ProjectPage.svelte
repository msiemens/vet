<script lang="ts">
  import type { ReviewFilter } from "../../generated/ReviewFilter";
  import { parseLines } from "../../lib/parseLines";

  let {
    filter,
    excluded,
    onsave,
  }: {
    /** The repository's filter, as the manifest carries it; undefined without an open repository. */
    filter: ReviewFilter | undefined;
    /** Changes the filter currently excludes; the backend computes them. */
    excluded: { lines: number; items: number };
    /** Saves a changed filter; resolves with the backend's message if it refused it. */
    onsave: (filter: ReviewFilter) => Promise<string | undefined>;
  } = $props();

  let error = $state<string>();

  /** A refused edit stays in its field with the message below; an accepted one comes back as the new manifest's filter. */
  async function save(change: Partial<ReviewFilter>) {
    if (filter) error = await onsave({ ...filter, ...change });
  }
</script>

{#if !filter}
  <p class="empty">Open a repository to edit its review filter.</p>
{:else}
  <form onsubmit={(e) => e.preventDefault()}>
    <p class="hint">
      What a review shows and counts in this repository. It applies to every changeset of it and never changes review
      state.
    </p>
    <label>
      <span>Review roots</span>
      <textarea
        class="mono"
        rows="2"
        aria-label="Review roots, one folder per line"
        value={filter.roots.join("\n")}
        onchange={(e) => save({ roots: parseLines(e.currentTarget.value) })}></textarea>
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
        aria-label="Include globs, one per line"
        value={filter.include.join("\n")}
        onchange={(e) => save({ include: parseLines(e.currentTarget.value) })}></textarea>
    </label>
    <label>
      <span>Exclude globs</span>
      <textarea
        class="mono"
        rows="3"
        aria-label="Exclude globs, one per line"
        value={filter.exclude.join("\n")}
        onchange={(e) => save({ exclude: parseLines(e.currentTarget.value) })}></textarea>
    </label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <p class="count">
      <span class="mono">{excluded.lines}</span> changed lines and <span class="mono">{excluded.items}</span> other changes
      excluded
    </p>
  </form>
{/if}

<style>
  form {
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
  p {
    margin: 0;
  }
  .hint {
    color: var(--text-3);
    font-size: 12px;
  }
  .empty {
    color: var(--text-3);
  }
  .error {
    color: var(--del-text);
  }
  .count {
    padding-top: 8px;
    border-top: 1px solid var(--line-soft);
    color: var(--text-2);
  }
</style>

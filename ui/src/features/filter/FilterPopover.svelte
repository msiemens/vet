<script module lang="ts">
  export interface ReviewFilter {
    roots: string[];
    hideBinary: boolean;
    include: string[];
    exclude: string[];
  }
</script>

<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { mockFilter } from "./mock";

  let {
    filter = $bindable(mockFilter),
    excluded,
  }: {
    filter?: ReviewFilter;
    /** Changes the filter currently excludes; the backend computes them. */
    excluded: { lines: number; items: number };
  } = $props();

  let open = $state(false);
  const lines = (text: string) =>
    text
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean);
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
        onchange={(e) => (filter.roots = lines(e.currentTarget.value))}></textarea>
    </label>
    <label class="check"><input type="checkbox" bind:checked={filter.hideBinary} />Hide binary files</label>
    <label>
      <span>Include globs</span>
      <textarea
        class="mono"
        rows="2"
        value={filter.include.join("\n")}
        onchange={(e) => (filter.include = lines(e.currentTarget.value))}></textarea>
    </label>
    <label>
      <span>Exclude globs</span>
      <textarea
        class="mono"
        rows="3"
        value={filter.exclude.join("\n")}
        onchange={(e) => (filter.exclude = lines(e.currentTarget.value))}></textarea>
    </label>
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
    color: var(--tx2);
  }
  .check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--tx);
  }
  textarea {
    resize: vertical;
    border: 1px solid var(--ln);
    border-radius: var(--radius);
    background: var(--ed);
    color: var(--tx);
    font-size: var(--fs-mono);
    padding: 4px 8px;
  }
  .count {
    margin: 0;
    padding-top: 8px;
    border-top: 1px solid var(--ln2);
    color: var(--tx2);
  }
</style>

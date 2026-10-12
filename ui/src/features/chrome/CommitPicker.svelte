<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { CommitInfo } from "../../generated/CommitInfo";
  import { searchCommits } from "./commitSearch";
  import { shortTime } from "../../lib/shortTime";

  let {
    commits,
    current,
    onpick,
  }: {
    /** Newest first. */
    commits: CommitInfo[];
    /** Full id of the commit now used as the base. */
    current: string;
    /** A commit's full id, or the typed text for a revision that is not in the list. */
    onpick: (id: string) => void;
  } = $props();

  let query = $state("");
  let input: HTMLInputElement;
  let list: HTMLElement;
  let highlighted = $state(0);

  const found = $derived(searchCommits(commits, query));
  const typed = $derived(query.trim());
  // Older commits and other revisions can still be typed in.
  const offerTyped = $derived(typed !== "" && !found.length);

  onMount(() => input.focus());

  /** Keeps the highlight in range as the list narrows, and the highlighted row in view. */
  async function highlight(index: number) {
    highlighted = Math.max(0, Math.min(index, found.length - 1));
    await tick();
    list.querySelector(".hl")?.scrollIntoView({ block: "nearest" });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      void highlight(highlighted + (e.key === "ArrowDown" ? 1 : -1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (found.length) onpick(found[highlighted].id);
      else if (offerTyped) onpick(typed);
    }
  }
</script>

<div class="picker">
  <input
    class="field"
    bind:this={input}
    bind:value={query}
    oninput={() => (highlighted = 0)}
    {onkeydown}
    type="text"
    placeholder="Search commits by id, subject or author"
    aria-label="Search commits"
    spellcheck="false"
  />
  <ul bind:this={list} role="listbox" aria-label="Commits">
    {#each found as commit, i (commit.id)}
      <li role="option" aria-selected={commit.id === current}>
        <button
          class="flat row"
          class:hl={i === highlighted}
          class:on={commit.id === current}
          type="button"
          tabindex="-1"
          onclick={() => onpick(commit.id)}
        >
          <span class="mono id">{commit.shortId}</span>
          <span class="subject">{commit.subject}</span>
          <span class="author">{commit.author}</span>
          <span class="when">{shortTime(commit.time, Date.now(), "detailed")}</span>
        </button>
      </li>
    {/each}
    {#if offerTyped}
      <li role="option" aria-selected="false">
        <button class="flat row" type="button" tabindex="-1" onclick={() => onpick(typed)}>
          <span class="subject">Use <span class="mono">{typed}</span> as a revision</span>
        </button>
      </li>
    {:else if !found.length}
      <li class="empty">No commits</li>
    {/if}
  </ul>
</div>

<style>
  .picker {
    width: 560px;
    display: flex;
    flex-direction: column;
  }
  input {
    margin: 8px;
  }
  input:focus-visible {
    outline-offset: -1px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 0 4px;
    max-height: 340px;
    overflow: hidden auto;
    border-top: 1px solid var(--line-soft);
  }
  .row {
    width: 100%;
    height: var(--h-row);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    text-align: left;
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
  /* The keyboard's choice; `.on` (the current base) is the toggled look. */
  .hl:not(.on) {
    background: var(--hover);
  }
  .id {
    flex: none;
    width: 7ch;
    font-size: var(--fs-mono-s);
    color: var(--text-2);
  }
  .subject {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .author,
  .when {
    flex: none;
    font-size: 12px;
    color: var(--text-3);
  }
  .author {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .when {
    width: 88px;
    text-align: right;
  }
  .empty {
    padding: 8px 12px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
</style>

<script lang="ts">
  import type { ConversationEntry } from "../../generated/ConversationEntry";
  import { shortTime } from "../../lib/shortTime";

  let { entries }: { entries: ConversationEntry[] } = $props();
</script>

{#if entries.length === 0}
  <p class="empty">Nothing was said on the pull request as a whole.</p>
{:else}
  <ul>
    {#each entries as entry, i (i)}
      <li>
        <div class="meta">
          <span class="author">{entry.author}</span>
          {#if entry.verdict}<span class="badge">{entry.verdict}</span>{/if}
          <span class="time">{shortTime(entry.at)}</span>
        </div>
        {#if entry.body}<div class="body selectable">{entry.body}</div>{/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .empty {
    margin: 12px 10px;
    color: var(--text-3);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    padding: 6px 10px;
    border-bottom: 1px solid var(--line-soft);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
    font-size: 11.5px;
  }
  .author {
    color: var(--text-2);
    font-weight: 600;
  }
  .body {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--fs-ctl);
  }
</style>

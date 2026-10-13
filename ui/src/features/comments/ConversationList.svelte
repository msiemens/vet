<script lang="ts">
  import type { ConversationEntry } from "../../generated/ConversationEntry";
  import { canSubmit, isSubmitKey } from "../../lib/composer";
  import { shortTime } from "../../lib/shortTime";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { platform } from "../../state/platform.svelte";
  import { threads } from "../../state/threads.svelte";
  import { type DraftComment, isOwnComment, verdictChip } from "./conversation";

  /** The platform's feed, oldest first, and the viewer's comments that wait for Submit review, which follow it. */
  let { entries, drafts }: { entries: ConversationEntry[]; drafts: DraftComment[] } = $props();

  /** The comment whose text is being edited: a feed entry's id, or `draft:` and a message's id. */
  let editing = $state<string>();
  let text = $state("");
  let saving = $state(false);

  function edit(key: string, body: string) {
    [editing, text] = [key, body];
  }

  async function save(change: (body: string) => Promise<boolean>) {
    if (!canSubmit(text, saving)) return;
    saving = true;
    const saved = await change(text.trim());
    saving = false;
    if (saved) editing = undefined;
  }
</script>

{#snippet form(change: (body: string) => Promise<boolean>)}
  <div class="edit">
    <!-- svelte-ignore a11y_autofocus -->
    <textarea
      class="field"
      rows="2"
      autofocus
      bind:value={text}
      onkeydown={(e) => {
        if (isSubmitKey(e)) {
          e.preventDefault();
          void save(change);
        } else if (e.key === "Escape") editing = undefined;
      }}></textarea>
    <div class="buttons">
      <button class="ctl small" type="button" onclick={() => (editing = undefined)}>Cancel</button>
      <button
        class="ctl primary small"
        type="button"
        disabled={!canSubmit(text, saving)}
        onclick={() => void save(change)}>Save</button
      >
    </div>
  </div>
{/snippet}

{#if entries.length === 0 && drafts.length === 0}
  <EmptyState align="start" text="Nothing was said on the pull request as a whole." />
{:else}
  <ul>
    {#each entries as entry (entry.id)}
      {@const chip = verdictChip(entry)}
      {@const own = isOwnComment(entry)}
      <li>
        <div class="meta">
          <span class="author" class:mine={own} title={entry.author}>{entry.author}</span>
          {#if chip}<span class="chip {chip.tone}">{chip.label}</span>{/if}
          <span class="time">{shortTime(entry.at, Date.now(), "detailed")}</span>
          <span class="grow"></span>
          {#if own && editing !== entry.id}
            <IconButton
              icon="edit"
              label="Edit the comment"
              size={20}
              iconSize={11}
              onclick={() => edit(entry.id, entry.body)}
            />
            <IconButton
              icon="close"
              label="Delete the comment"
              size={20}
              iconSize={11}
              onclick={() => void platform.deleteComment(entry.id)}
            />
          {/if}
        </div>
        {#if editing === entry.id}
          {@render form((body) => platform.editComment(entry.id, body))}
        {:else if entry.body}
          <div class="body selectable">{entry.body}</div>
        {/if}
      </li>
    {/each}
    {#each drafts as { message } (message.id)}
      {@const key = `draft:${message.id}`}
      <li>
        <div class="meta">
          <span class="author mine" title={message.author}>{message.author}</span>
          <span class="badge">Draft</span>
          <span class="time">{shortTime(message.at, Date.now(), "detailed")}</span>
          <span class="grow"></span>
          {#if editing !== key}
            <IconButton
              icon="edit"
              label="Edit the comment"
              size={20}
              iconSize={11}
              onclick={() => edit(key, message.body)}
            />
            {#if message.state.type !== "posting"}
              <IconButton
                icon="close"
                label="Discard the draft"
                size={20}
                iconSize={11}
                onclick={() => void threads.removeMessage(message.id)}
              />
            {/if}
          {/if}
        </div>
        {#if editing === key}
          {@render form((body) => threads.editMessage(message.id, body))}
        {:else}
          <div class="body selectable draft">{message.body}</div>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
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
    min-height: 20px;
    color: var(--text-3);
    font-size: 11.5px;
  }
  /* The author's name gives way before the chip, the time and the buttons do. */
  .meta > * {
    flex: none;
    white-space: nowrap;
  }
  .meta > .author {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta > .grow {
    flex: 1 0 0;
  }
  .author {
    color: var(--text-2);
    font-weight: 600;
  }
  .author.mine {
    color: var(--accent);
  }
  .grow {
    flex: 1;
  }
  .body {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--fs-ctl);
  }
  /* A comment that waits for Submit review is shaded, as in a thread card. */
  .draft {
    padding: 0 6px;
    background: color-mix(in srgb, var(--note) 10%, transparent);
    border-radius: var(--radius);
  }
  /* A review's verdict, coloured like the queue's status chips. */
  .chip {
    padding: 1px 6px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--tone) 16%, transparent);
    color: var(--tone);
    line-height: 1.2;
    white-space: nowrap;
  }
  .approved {
    --tone: var(--status-approved);
  }
  .changes {
    --tone: var(--status-changes);
  }
  .commented {
    --tone: var(--status-draft);
  }
  .edit {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 2px;
  }
  textarea {
    width: 100%;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .small {
    height: 20px;
    padding: 0 8px;
    font-size: var(--fs-mono-s);
  }
</style>

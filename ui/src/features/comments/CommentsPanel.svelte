<script lang="ts">
  import { onMount } from "svelte";
  import type { Thread } from "../../generated/Thread";
  import { canSubmit, isSubmitKey } from "../../lib/composer";
  import { movedTitle } from "../../lib/movedTitle";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { threads } from "../../state/threads.svelte";
  import { attachMenu, threadCommands } from "./commentCommands";
  import { candidates, detachedReason, groupThreads, placeLabel, preview } from "./groups";

  let { onselect }: { onselect: (thread: Thread) => void } = $props();

  const groups = $derived(groupThreads(threads.items));
  const empty = $derived(threads.items.length === 0);

  let composing = $state(false);
  let body = $state("");
  let sending = $state(false);

  onMount(() => commands.register(...threadCommands({ attach: (thread, at) => void threads.attach(thread, at) })));

  async function post() {
    if (threads.revision === undefined || !canSubmit(body, sending)) return;
    sending = true;
    const sent = await threads.create(threads.revision, { type: "changeset" }, body.trim());
    sending = false;
    if (sent) [body, composing] = ["", false];
  }
</script>

{#snippet row(thread: Thread, showPath: boolean)}
  <li class:resolved={thread.status === "resolved"}>
    <button class="flat main" type="button" onclick={() => onselect(thread)}>
      <span class="status">
        {#if thread.status === "resolved"}<Icon name="check" size={12} stroke={2.2} />{:else}<Icon
            name="comments"
            size={12}
          />{/if}
      </span>
      <span class="text">
        <span class="preview">{preview(thread)}</span>
        <span class="mono meta">
          {#if showPath}{thread.path}
          {/if}{placeLabel(thread)} · {thread.messages.length}
          {thread.messages.length === 1 ? "message" : "messages"}
        </span>
      </span>
    </button>
    {@render actions(thread)}
  </li>
{/snippet}

{#snippet actions(thread: Thread)}
  {#if thread.anchor?.type === "inferred"}
    <div class="actions">
      <span class="badge badge-moved" title={movedTitle(thread)}>moved</span>
      <span class="grow"></span>
      <button class="ctl small" type="button" onclick={() => void threads.confirm(thread.id)}>Confirm</button>
    </div>
  {:else if thread.anchor?.type === "ambiguous" || thread.anchor?.type === "stale"}
    <div class="actions">
      <span class="reason">{detachedReason(thread)}</span>
      <span class="grow"></span>
      {#if candidates(thread).length}
        <button
          class="ctl small"
          type="button"
          onclick={(e) => void contextMenus.open(attachMenu(thread), e.clientX, e.clientY, { thread })}
        >
          Attach to {candidates(thread).length}
        </button>
      {/if}
      <button class="ctl small" type="button" onclick={() => void threads.remove(thread.id)}>Discard</button>
    </div>
  {/if}
{/snippet}

{#snippet group(title: string, list: Thread[], showPath: boolean, id: string)}
  <section aria-label={title} data-group={id}>
    <div class="sh">
      <Icon name="open" size={10} />
      <span class="title" class:mono={id.startsWith("file:")}>{title}</span>
      <span class="mono n">{list.length}</span>
    </div>
    <ul>
      {#each list as thread (thread.id)}{@render row(thread, showPath)}{/each}
    </ul>
  </section>
{/snippet}

<aside aria-label="Comments">
  <div class="sh sh-bar">
    <span>Comments</span>
    <span class="grow"></span>
    <IconButton
      icon="comments"
      label="Comment on the changeset"
      size={22}
      iconSize={13}
      onclick={() => (composing = !composing)}
    />
  </div>
  {#if composing}
    <div class="compose">
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        class="field"
        rows="3"
        placeholder="Comment on the changeset…"
        autofocus
        bind:value={body}
        onkeydown={(e) => {
          if (isSubmitKey(e)) {
            e.preventDefault();
            void post();
          } else if (e.key === "Escape") composing = false;
        }}></textarea>
      <div class="buttons">
        <button class="ctl" type="button" onclick={() => (composing = false)}>Cancel</button>
        <button class="ctl primary" type="button" disabled={!canSubmit(body, sending)} onclick={() => void post()}>
          Comment
        </button>
      </div>
    </div>
  {/if}
  {#if empty && !composing}
    <p class="empty">
      No comments yet. Select lines and press {commands.binding("review.comment")?.label}, or use a hunk's comment
      button.
    </p>
  {/if}
  {#if groups.detached.length}{@render group("Detached", groups.detached, true, "detached")}{/if}
  {#if groups.moved.length}{@render group("Moved", groups.moved, true, "moved")}{/if}
  {#if groups.changeset.length}{@render group("Changeset", groups.changeset, false, "changeset")}{/if}
  {#each groups.files as file (file.path)}
    {@render group(file.path, file.threads, false, `file:${file.path}`)}
  {/each}
</aside>

<style>
  aside {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden auto;
  }
  .empty {
    margin: 12px 10px;
    color: var(--text-3);
  }
  .compose {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--line-soft);
  }
  textarea {
    width: 100%;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .sh .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  ul {
    margin: 0;
    padding: 0 0 6px;
    list-style: none;
    border-bottom: 1px solid var(--line-soft);
  }
  li.resolved .preview,
  li.resolved .status {
    color: var(--text-3);
  }
  .main {
    width: 100%;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 3px 10px 3px 26px;
    text-align: left;
    font-size: var(--fs-ctl);
  }
  .status {
    flex: none;
    display: inline-flex;
    height: 18px;
    align-items: center;
    margin-left: -16px;
    color: var(--text-2);
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .preview {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 3px 26px;
  }
  .small {
    height: 20px;
    padding: 0 8px;
    font-size: var(--fs-mono-s);
  }
  .reason {
    font-size: 11.5px;
    color: var(--text-3);
  }
</style>

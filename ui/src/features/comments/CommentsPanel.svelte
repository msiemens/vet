<script lang="ts">
  import type { Thread } from "../../generated/Thread";
  import { KEYS } from "../../lib/keymap";
  import ContextMenu from "../../lib/ui/ContextMenu.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { threads } from "../../state/threads.svelte";
  import { candidateLabel, candidates, detachedReason, groupThreads, placeLabel, preview } from "./groups";

  let { onselect }: { onselect: (thread: Thread) => void } = $props();

  const groups = $derived(groupThreads(threads.items));
  const empty = $derived(threads.items.length === 0);

  let composing = $state(false);
  let body = $state("");
  let sending = $state(false);
  /** "Attach to N" lists the candidates of `thread` at the pointer. */
  let menu = $state<{ thread: Thread; x: number; y: number }>();

  async function post() {
    if (threads.revision === undefined || !body.trim() || sending) return;
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
      <span class="badge moved" title="Found by its surrounding lines, not where it was put">moved</span>
      <span class="grow"></span>
      <button class="ctl small" type="button" onclick={() => void threads.confirm(thread.id)}>Confirm</button>
    </div>
  {:else if thread.anchor?.type === "ambiguous" || thread.anchor?.type === "stale"}
    <div class="actions">
      <span class="reason">{detachedReason(thread)}</span>
      <span class="grow"></span>
      {#if candidates(thread).length}
        <button class="ctl small" type="button" onclick={(e) => (menu = { thread, x: e.clientX, y: e.clientY })}>
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
  <div class="head">
    <span class="title">Comments</span>
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
        rows="3"
        placeholder="Comment on the changeset…"
        autofocus
        bind:value={body}
        onkeydown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void post();
          } else if (e.key === "Escape") composing = false;
        }}></textarea>
      <div class="buttons">
        <button class="ctl" type="button" onclick={() => (composing = false)}>Cancel</button>
        <button class="ctl primary" type="button" disabled={!body.trim() || sending} onclick={() => void post()}>
          Comment
        </button>
      </div>
    </div>
  {/if}
  {#if empty && !composing}
    <p class="empty">
      No comments yet. Select lines and press {KEYS.commentHere.label}, or use a hunk's comment button.
    </p>
  {/if}
  {#if groups.detached.length}{@render group("Detached", groups.detached, true, "detached")}{/if}
  {#if groups.moved.length}{@render group("Moved", groups.moved, true, "moved")}{/if}
  {#if groups.changeset.length}{@render group("Changeset", groups.changeset, false, "changeset")}{/if}
  {#each groups.files as file (file.path)}
    {@render group(file.path, file.threads, false, `file:${file.path}`)}
  {/each}
</aside>

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    items={candidates(menu.thread).map((at, i) => ({ id: String(i), label: candidateLabel(at) }))}
    onselect={(id) => menu && void threads.attach(menu.thread.id, Number(id))}
    onclose={() => (menu = undefined)}
  />
{/if}

<style>
  aside {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden auto;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-ctl);
    padding: 0 6px 0 10px;
    flex: none;
    border-bottom: 1px solid var(--line-soft);
  }
  .head .title {
    font-weight: 600;
    font-size: var(--fs-ctl);
  }
  .grow {
    flex: 1;
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
    box-sizing: border-box;
    width: 100%;
    padding: 4px 6px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-editor);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-ctl);
    resize: none;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .sh {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-ctl);
    padding: 0 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
  }
  .sh .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .n {
    font-weight: 400;
    color: var(--text-3);
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
  .badge {
    height: 16px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    padding: 0 6px;
    border: 1px solid var(--note);
    border-radius: 8px;
    color: var(--note);
    font-size: 11px;
  }
</style>

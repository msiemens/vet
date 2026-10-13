<script lang="ts">
  import type { Thread } from "../../generated/Thread";
  import { canSubmit, isSubmitKey } from "../../lib/composer";
  import { movedLabel, movedTitle } from "../../lib/movedTitle";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import GroupHeader from "../../lib/ui/GroupHeader.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { changeset } from "../../state/changeset.svelte";
  import { commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { threads } from "../../state/threads.svelte";
  import PanelTabs from "../../lib/ui/PanelTabs.svelte";
  import { platform } from "../../state/platform.svelte";
  import { attachMenu } from "./commentCommands";
  import { changesetThreads, draftComments, reviewThreads } from "./conversation";
  import ConversationList from "./ConversationList.svelte";
  import { RESOLVED_SHOWN, candidates, detachedReason, draftCount, groupThreads, preview, shown } from "./groups";
  import PullRequestBlock from "./PullRequestBlock.svelte";
  import SubmitReview from "./SubmitReview.svelte";
  import ThreadCards from "./ThreadCards.svelte";
  import ThreadPlace from "./ThreadPlace.svelte";

  let {
    onselect,
    card,
  }: {
    onselect: (thread: Thread) => void;
    /** Draws a thread as the diff's card, for the threads on the pull request as a whole where they are threads. */
    card: (thread: Thread, open: boolean, toggle: () => void) => HTMLElement;
  } = $props();

  const info = $derived(platform.info);
  const listed = $derived(reviewThreads(threads.items));
  const groups = $derived(groupThreads(listed));
  const empty = $derived(listed.length === 0);
  const drafts = $derived(draftComments(threads.items));
  const onWhole = $derived(changesetThreads(threads.items));
  const conversationCount = $derived(
    info?.traits.conversationFeed ? info.conversation.length + drafts.length : onWhole.length,
  );
  type Tab = typeof platform.commentsTab;
  const pick = (id: Tab) => (platform.commentsTab = id);
  // A changeset that is no pull request has no conversation to switch to.
  const showing = $derived(info ? platform.commentsTab : "review");

  type GroupId = "conflicts" | "open" | "resolved";
  let folded = $state<GroupId[]>([]);
  let showAllResolved = $state(false);
  const fold = (id: GroupId) => (folded = folded.includes(id) ? folded.filter((f) => f !== id) : [...folded, id]);

  // The draft is the store's, so the Comment on Changeset command can open it and switching tool windows keeps it.
  const draft = platform.changesetComment;
  let sending = $state(false);
  /** A comment on the changeset is written below its conversation, where it will be; without a pull request, below the threads. */
  const composeHere = $derived(draft.composing && showing === (info ? "conversation" : "review"));

  async function post() {
    // The panel is no view of files: it comments on the changeset as the manifest on screen is of.
    const onScreen = changeset.currentShown();
    if (!onScreen || !canSubmit(draft.body, sending)) return;
    sending = true;
    const sent = await threads.create(onScreen, { type: "changeset" }, draft.body.trim());
    sending = false;
    if (sent) [draft.body, draft.composing] = ["", false];
  }
</script>

{#snippet row(thread: Thread)}
  {@const author = thread.messages[0]?.author}
  <li class:resolved={thread.status === "resolved"}>
    <button class="flat main" type="button" onclick={() => onselect(thread)}>
      <span class="text">
        <span class="top">
          <span class="mono where"><ThreadPlace {thread} /></span>
          {#if author}<span class="author" class:mine={thread.messages[0]?.mine} title={author}
              >{thread.messages[0]?.mine ? "You" : author}</span
            >{/if}
          <span class="grow"></span>
          {#if draftCount(thread)}<span class="draft">{draftCount(thread)} draft</span>{/if}
          {#if thread.status === "resolved" && thread.platform?.resolvedBy}
            <span class="by">by {thread.platform.resolvedBy}</span>
          {/if}
          {#if thread.platform}
            <span
              class="sync"
              class:local={!thread.platform.posted}
              title={thread.platform.posted ? "On the platform" : "Only in Vet until it is posted"}
            >
              {#if thread.platform.posted}<Icon
                  name={info?.platform.kind === "azureDevOps" ? "azureDevOps" : "github"}
                  size={11}
                />{/if}
            </span>
          {/if}
        </span>
        <span class="preview">{preview(thread)}</span>
      </span>
    </button>
    {@render actions(thread)}
  </li>
{/snippet}

{#snippet actions(thread: Thread)}
  {#if thread.anchor?.type === "inferred"}
    <div class="actions">
      <span class="badge badge-moved" title={movedTitle(thread)}>{movedLabel(thread)}</span>
      <span class="grow"></span>
      <button class="ctl small" type="button" onclick={() => void threads.confirm(thread.id)}>Confirm</button>
    </div>
  {:else if thread.anchor?.type === "ambiguous" || thread.anchor?.type === "stale"}
    <div class="actions">
      <span class="reason" title={detachedReason(thread)}>{detachedReason(thread)}</span>
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
      <button
        class="ctl small"
        type="button"
        onclick={() => commands.run("thread.delete", commands.context({ thread }))}
        >{commands.label("thread.delete")}</button
      >
    </div>
  {/if}
{/snippet}

{#snippet group(title: string, list: Thread[], id: GroupId)}
  {@const cut = shown(list, id === "resolved" && showAllResolved, RESOLVED_SHOWN)}
  <section aria-label={title} data-group={id}>
    <GroupHeader open={!folded.includes(id)} count={list.length} onclick={() => fold(id)}>
      <span class:conflict={id === "conflicts"}>{title}</span>
    </GroupHeader>
    {#if !folded.includes(id)}
      <ul>
        {#each id === "resolved" ? cut.rows : list as thread (thread.id)}{@render row(thread)}{/each}
      </ul>
      {#if id === "resolved" && cut.more > 0}
        <button class="flat more" type="button" onclick={() => (showAllResolved = true)}>
          <Icon name="closed" size={10} />{cut.more} more
        </button>
      {/if}
    {/if}
  </section>
{/snippet}

<aside aria-label="Comments">
  <div class="sh sh-bar">
    <span>Comments</span>
    <span class="grow"></span>
    <IconButton
      icon="comments"
      label={commands.label("review.commentOnChangeset")}
      title={commands.hint("review.commentOnChangeset")}
      disabled={!commands.isEnabled("review.commentOnChangeset")}
      size={22}
      iconSize={13}
      onclick={() => commands.run("review.commentOnChangeset")}
    />
  </div>
  <ScrollArea>
    <div class="content">
      {#if info}
        <PullRequestBlock {info} />
        <SubmitReview {info} />
        <PanelTabs
          label="Pull request"
          value={showing}
          onchange={pick}
          tabs={[
            { id: "review", label: "Review", count: listed.length },
            { id: "conversation", label: "Conversation", count: conversationCount },
          ]}
        />
      {/if}
      {#if info && showing === "conversation"}
        {#if info.traits.conversationFeed}
          <ConversationList entries={info.conversation} {drafts} />
        {:else if onWhole.length}
          <ThreadCards list={onWhole} {card} />
        {:else if !composeHere}
          <EmptyState align="start" text="Nothing was said on the pull request as a whole." />
        {/if}
      {:else}
        <!-- Without a pull request there is no Conversation tab: the changeset's own threads come first here. -->
        {#if !info && onWhole.length}<ThreadCards list={onWhole} {card} />{/if}
        {#if empty && (info !== undefined || !onWhole.length) && !composeHere}
          <EmptyState
            align="start"
            text="No comments yet. Select lines and press {commands.binding('review.comment')
              ?.label}, or use a hunk's comment button."
          />
        {/if}
        {#if groups.conflicts.length}{@render group("Conflicts", groups.conflicts, "conflicts")}{/if}
        {#if groups.open.length}{@render group("Open", groups.open, "open")}{/if}
        {#if groups.resolved.length}{@render group("Resolved", groups.resolved, "resolved")}{/if}
      {/if}
      {#if composeHere}
        <div
          class="compose"
          {@attach (box: HTMLElement) => {
            box.scrollIntoView({ block: "nearest" });
          }}
        >
          <!-- svelte-ignore a11y_autofocus -->
          <textarea
            class="field"
            rows="3"
            placeholder="Comment on the changeset…"
            autofocus
            bind:value={draft.body}
            onkeydown={(e) => {
              if (isSubmitKey(e)) {
                e.preventDefault();
                void post();
              } else if (e.key === "Escape") draft.composing = false;
            }}></textarea>
          <div class="buttons">
            <button class="ctl" type="button" onclick={() => (draft.composing = false)}>Cancel</button>
            <button
              class="ctl primary"
              type="button"
              disabled={!canSubmit(draft.body, sending)}
              onclick={() => void post()}
            >
              Comment
            </button>
          </div>
        </div>
      {/if}
    </div>
  </ScrollArea>
</aside>

<style>
  aside {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .content {
    display: flex;
    flex-direction: column;
  }
  .compose {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
  }
  textarea {
    width: 100%;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  ul {
    margin: 0;
    padding: 0 0 6px;
    list-style: none;
    border-bottom: 1px solid var(--line-soft);
  }
  li + li {
    border-top: 1px solid var(--line-soft);
  }
  li.resolved .preview {
    color: var(--text-3);
  }
  .conflict {
    color: var(--warn);
  }
  .main {
    width: 100%;
    display: flex;
    align-items: flex-start;
    padding: 3px 10px 3px 26px;
    text-align: left;
    font-size: var(--fs-ctl);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: var(--fs-mono-s);
  }
  /* Nothing in the line wraps: the file and the author give way with an ellipsis, the rest keeps its size. */
  .top > * {
    flex: none;
    white-space: nowrap;
  }
  .top > .where,
  .top > .author {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .top > .author {
    flex-shrink: 1000;
  }
  .top > .grow {
    flex: 1 0 0;
  }
  .author {
    color: var(--note);
  }
  .author.mine {
    color: var(--accent);
  }
  .draft {
    color: var(--note);
  }
  .by {
    color: var(--text-3);
  }
  .sync {
    display: inline-flex;
    align-items: center;
    color: var(--ok);
  }
  .sync.local {
    width: 9px;
    height: 9px;
    box-sizing: border-box;
    border: 1px solid var(--text-3);
    border-radius: 2px;
  }
  .preview {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .more {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 10px;
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 3px 26px;
  }
  .reason {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    color: var(--text-3);
  }
</style>

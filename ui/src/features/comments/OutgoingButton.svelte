<script lang="ts">
  import { onDestroy, tick, untrack } from "svelte";
  import type { Thread } from "../../generated/Thread";
  import { SUBMIT_KEY } from "../../lib/composer";
  import Icon from "../../lib/ui/Icon.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import Spinner from "../../lib/ui/Spinner.svelte";
  import { verdictOf } from "../../lib/verdicts";
  import { commands } from "../../state/commands";
  import { platform } from "../../state/platform.svelte";
  import { threads } from "../../state/threads.svelte";
  import { firstLine } from "./groups";
  import {
    canPost,
    destination,
    outgoingComments,
    outgoingKey,
    outgoingTitle,
    postLabel,
    summarizeOutgoing,
    voteChoices,
    voteText,
  } from "./outgoing";
  import PostingControls from "./PostingControls.svelte";
  import { sendComments } from "./posting";
  import { draftsToSubmit, submitBlock, voteWaits } from "./pullRequest";
  import ThreadPlace from "./ThreadPlace.svelte";

  /** The toolbar's count of the viewer's comments and staged vote that the platform does not have yet; a click lists them, and posts them. */
  let { onopen }: { onopen: (thread: Thread) => void } = $props();

  // The list is open in the store, so the Post Comments command can open it.
  onDestroy(() => (platform.outgoingOpen = false));
  $effect(() => {
    if (!platform.outgoingOpen) platform.votePicking = false;
  });
  const info = $derived(platform.info);
  const list = $derived(outgoingComments(threads.items));
  const vote = $derived(info?.pendingVote ?? null);
  const summary = $derived(summarizeOutgoing(list, vote));
  const busy = $derived(platform.sending || summary.posting);
  const comments = $derived(draftsToSubmit(threads.items));
  const ready = $derived(comments + (voteWaits(vote) ? 1 : 0));
  const blocked = $derived(info && submitBlock(info.platform.connection));
  const title = $derived(outgoingTitle(summary, vote !== null));

  function show(thread: Thread) {
    platform.outgoingOpen = false;
    onopen(thread);
  }

  async function pick(id: string) {
    platform.outgoingOpen = false;
    const verdict = verdictOf(id);
    if (verdict) await platform.stageVote(verdict);
  }

  const postable = $derived(canPost(ready, busy, blocked));

  async function send() {
    if (info && (await sendComments(info))) platform.outgoingOpen = false;
  }

  function onkeydown(e: KeyboardEvent) {
    if (outgoingKey(e, platform.votePicking) !== "post") return;
    e.preventDefault();
    if (postable) void send();
  }

  // Keys work as soon as the list opens, and the focus goes back where it was when it closes.
  let sheet: HTMLElement | undefined;
  let returnTo: Element | null = null;
  // Before the list renders: the vote picker's menu takes the focus as it mounts.
  $effect.pre(() => {
    if (platform.outgoingOpen) returnTo ??= document.activeElement;
  });
  $effect(() => {
    if (!platform.outgoingOpen) {
      // A focus that moved on by itself (to a thread that was opened) stays there.
      if (returnTo instanceof HTMLElement && returnTo.isConnected && document.activeElement === document.body) {
        returnTo.focus();
      }
      returnTo = null;
      return;
    }
    // The vote picker's menu takes the focus itself.
    if (untrack(() => platform.votePicking)) return;
    void tick().then(() => {
      const target = sheet?.querySelector<HTMLElement>("textarea, button.primary:enabled") ?? sheet;
      target?.focus();
    });
  });
</script>

<Popover bind:open={platform.outgoingOpen} align="right">
  {#snippet trigger(toggle)}
    <button
      class="ctl status"
      class:failed={summary.failed > 0}
      type="button"
      disabled={summary.count === 0}
      aria-expanded={platform.outgoingOpen}
      {title}
      aria-label={title}
      onclick={toggle}
    >
      {#if busy}<Spinner />{:else}<Icon name="outgoing" size={12} />{/if}
      <span class="mono">{summary.failed > 0 ? summary.failed : summary.count}</span>
    </button>
  {/snippet}
  {#if platform.votePicking}
    {#if info}<Menu items={voteChoices(info)} onselect={(verdict) => void pick(verdict)} />{/if}
  {:else}
    <div class="outgoing" role="presentation" tabindex="-1" bind:this={sheet} {onkeydown}>
      <ul>
        {#each list as { thread, message } (message.id)}
          <li>
            <button class="flat row" type="button" onclick={() => show(thread)}>
              <span class="mono where"><ThreadPlace {thread} changeset="on the pull request" /></span>
              <span class="text">{firstLine(message.body)}</span>
              {#if message.state.type === "posting"}<span class="note">posting…</span>{/if}
            </button>
            {#if message.state.type === "failed"}
              <div class="failure">
                <span class="error">{message.state.error}</span>
                <button
                  class="ctl small"
                  type="button"
                  disabled={busy}
                  onclick={() => void threads.removeMessage(message.id)}>Discard draft</button
                >
                <button
                  class="ctl small"
                  type="button"
                  disabled={busy || blocked !== undefined}
                  onclick={() => void send()}>Retry</button
                >
              </div>
            {/if}
          </li>
        {/each}
        {#if info && vote}
          <li>
            <div class="row">
              <span class="text">{voteText(vote)}</span>
              {#if vote.state.type === "posting"}<span class="note">posting…</span>{/if}
            </div>
            {#if vote.state.type === "failed"}
              <div class="failure">
                <span class="error">{vote.state.error}</span>
                <button class="ctl small" type="button" disabled={busy} onclick={() => void platform.discardVote()}
                  >Discard vote</button
                >
                <button
                  class="ctl small"
                  type="button"
                  disabled={busy || blocked !== undefined}
                  onclick={() => void send()}>Retry</button
                >
              </div>
            {/if}
          </li>
        {/if}
      </ul>
      {#if info}
        <div class="post">
          <PostingControls {info} part="review" />
          <PostingControls {info} part="vote" disabled={blocked !== undefined} />
          <div class="send">
            <span class="where-to">{destination(info)}</span>
            {#if blocked}
              <button class="link" type="button" onclick={() => commands.run("app.accounts")}>Sign in…</button>
            {/if}
            <button
              class="ctl primary"
              type="button"
              disabled={!postable}
              title={blocked ?? commands.keyLabel(SUBMIT_KEY)}
              onclick={() => void send()}>{postLabel(comments, voteWaits(vote))}</button
            >
          </div>
        </div>
      {/if}
    </div>
  {/if}
</Popover>

<style>
  .status {
    margin-right: 6px;
  }
  .failed {
    color: var(--severity-error);
  }
  .outgoing {
    outline: none;
    width: 380px;
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    font-size: var(--fs-ctl);
  }
  ul {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: 4px 0;
    list-style: none;
  }
  li + li {
    border-top: 1px solid var(--line-soft);
  }
  .row {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 5px 12px;
    text-align: left;
  }
  .where {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 60%;
    display: flex;
    font-size: var(--fs-mono-s);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    color: var(--text-3);
  }
  .failure {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px 5px;
  }
  .error {
    flex: 1;
    min-width: 0;
    color: var(--severity-error);
  }
  .post {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px;
    border-top: 1px solid var(--line);
  }
  .send {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
  .where-to {
    flex: 1;
    color: var(--text-3);
  }
</style>

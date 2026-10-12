<script lang="ts">
  import type { Thread } from "../../generated/Thread";
  import Icon from "../../lib/ui/Icon.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { platform } from "../../state/platform.svelte";
  import { threads } from "../../state/threads.svelte";
  import { firstLine } from "./groups";
  import {
    destination,
    outgoingComments,
    outgoingPlace,
    outgoingTitle,
    postLabel,
    summarizeOutgoing,
    voteText,
  } from "./outgoing";
  import PostingControls from "./PostingControls.svelte";
  import { sendComments } from "./posting";
  import { draftsToSubmit, submitBlock, voteWaits } from "./pullRequest";

  /** The toolbar's count of the viewer's comments and staged vote that the platform does not have yet; a click lists them, and posts them. */
  let { onopen }: { onopen: (thread: Thread) => void } = $props();

  let open = $state(false);
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
    open = false;
    onopen(thread);
  }

  async function send() {
    if (info && (await sendComments(info))) open = false;
  }
</script>

<Popover bind:open align="right">
  {#snippet trigger(toggle)}
    <button
      class="ctl status"
      class:failed={summary.failed > 0}
      type="button"
      disabled={summary.count === 0}
      aria-expanded={open}
      {title}
      aria-label={title}
      onclick={toggle}
    >
      {#if busy}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name="outgoing" size={12} />{/if}
      <span class="mono">{summary.failed > 0 ? summary.failed : summary.count}</span>
    </button>
  {/snippet}
  <div class="outgoing">
    <ul>
      {#each list as { thread, message } (message.id)}
        <li>
          <button class="flat row" type="button" onclick={() => show(thread)}>
            <span class="mono where">{outgoingPlace(thread)}</span>
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
                onclick={() => void threads.removeMessage(message.id)}>Discard</button
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
            <span class="text">{voteText(info, vote)}</span>
            {#if vote.state.type === "posting"}<span class="note">posting…</span>{/if}
          </div>
          {#if vote.state.type === "failed"}
            <div class="failure">
              <span class="error">{vote.state.error}</span>
              <button class="ctl small" type="button" disabled={busy} onclick={() => void platform.discardVote()}
                >Discard</button
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
            <button class="flat link" type="button" onclick={() => platform.openAccounts()}>Sign in…</button>
          {/if}
          <button
            class="ctl primary"
            type="button"
            disabled={ready === 0 || busy || blocked !== undefined}
            title={blocked}
            onclick={() => void send()}>{postLabel(comments, voteWaits(vote))}</button
          >
        </div>
      </div>
    {/if}
  </div>
</Popover>

<style>
  .status {
    margin-right: 6px;
  }
  .failed {
    color: var(--severity-error);
  }
  .spinner {
    width: 10px;
    height: 10px;
    box-sizing: border-box;
    border: 1.5px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .outgoing {
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
    flex: none;
    color: var(--text-2);
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
  .small {
    height: 20px;
    padding: 0 8px;
    font-size: var(--fs-mono-s);
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
  .link {
    color: var(--accent);
  }
</style>

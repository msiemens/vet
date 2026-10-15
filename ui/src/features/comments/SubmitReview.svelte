<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import { commands } from "../../state/commands";
  import { platform } from "../../state/platform.svelte";
  import PostingControls from "./PostingControls.svelte";
  import { sendComments } from "./posting";
  import { submitBlock } from "./pullRequest";

  let { info }: { info: PullRequestInfo } = $props();

  // The staged vote is posted with the comments, so it counts among them.
  const drafts = $derived(info.outgoing.comments + (info.outgoing.vote ? 1 : 0));
  const blocked = $derived(submitBlock(info.platform.connection));
  const form = platform.review;
</script>

<div class="submit">
  {#if info.traits.batchedReview}
    <button
      class="ctl primary"
      type="button"
      disabled={blocked !== undefined}
      title={blocked}
      aria-expanded={form.open}
      onclick={() => (form.open = !form.open)}
    >
      Submit review{#if drafts > 0}<span class="drafts">{drafts}</span>{/if}
    </button>
  {:else}
    <button
      class="ctl primary"
      type="button"
      disabled={blocked !== undefined || drafts === 0 || platform.sending}
      title={blocked}
      onclick={() => void sendComments(info)}
    >
      Post{#if drafts > 0}<span class="drafts">{drafts}</span>{/if}
    </button>
  {/if}
  <PostingControls {info} part="vote" disabled={blocked !== undefined} />
  {#if blocked}
    <button class="link" type="button" onclick={() => commands.run("app.accounts")}>Sign in…</button>
  {/if}
  {#if info.traits.batchedReview && form.open && !blocked}
    <div class="form">
      <PostingControls {info} part="review" />
      <div class="buttons">
        <button class="ctl" type="button" onclick={() => (form.open = false)}>Cancel</button>
        <button class="ctl primary" type="button" disabled={platform.sending} onclick={() => void sendComments(info)}
          >Submit</button
        >
      </div>
    </div>
  {/if}
</div>

<style>
  .submit {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line-soft);
  }
  .drafts {
    margin-left: 6px;
    font-family: var(--font-mono);
  }
  .form {
    flex: 1 0 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
</style>

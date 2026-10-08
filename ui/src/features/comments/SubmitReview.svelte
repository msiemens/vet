<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import { session } from "../../state/session.svelte";
  import { platform } from "../../state/platform.svelte";
  import { threads } from "../../state/threads.svelte";
  import { draftsToSubmit, submitBlock, waitingNote } from "./pullRequest";

  let { info }: { info: PullRequestInfo } = $props();

  const drafts = $derived(draftsToSubmit(threads.items));
  const blocked = $derived(submitBlock(info.platform.connection));

  // The form's state is the store's, so it outlives this panel.
  const form = platform.review;
  const verdict = $derived(form.verdict ?? info.traits.verdicts.at(0)?.id ?? "");
  let sending = $state(false);

  async function submit() {
    sending = true;
    const done = await platform.submit(verdict, form.body);
    sending = false;
    // What stopped it is told by the backend's notification.
    if (done?.problem !== null) return;
    [form.open, form.body] = [false, ""];
    session.notice = done.skipped > 0 ? `Review submitted. ${waitingNote(done.skipped)}` : "Review submitted";
  }

  async function post() {
    sending = true;
    const done = await platform.post();
    sending = false;
    if (done?.problem !== null) return;
    const posted = `${done.posted} ${done.posted === 1 ? "comment" : "comments"} posted`;
    session.notice = done.skipped > 0 ? `${posted}. ${waitingNote(done.skipped)}` : posted;
  }
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
      Submit review{#if drafts > 0}<span class="n">{drafts}</span>{/if}
    </button>
  {:else}
    <button
      class="ctl primary"
      type="button"
      disabled={blocked !== undefined || drafts === 0 || sending}
      title={blocked}
      onclick={() => void post()}
    >
      Post{#if drafts > 0}<span class="n">{drafts}</span>{/if}
    </button>
  {/if}
  {#if info.traits.verdictSeparate}
    <label class="vote">
      <span>Vote</span>
      <select
        class="field"
        disabled={blocked !== undefined}
        value={info.viewerVerdict ?? ""}
        onchange={(e) => void platform.vote(e.currentTarget.value)}
      >
        {#if !info.viewerVerdict}<option value="" disabled>No vote</option>{/if}
        {#each info.traits.verdicts as v (v.id)}<option value={v.id}>{v.label}</option>{/each}
      </select>
    </label>
  {/if}
  {#if blocked}
    <button class="flat link" type="button" onclick={() => platform.openAccounts()}>Sign in…</button>
  {/if}
  {#if info.traits.batchedReview && form.open && !blocked}
    <div class="form">
      <label>
        <span>Verdict</span>
        <select class="field" value={verdict} onchange={(e) => (form.verdict = e.currentTarget.value)}>
          {#each info.traits.verdicts as v (v.id)}<option value={v.id}>{v.label}</option>{/each}
        </select>
      </label>
      <textarea class="field" rows="3" placeholder="Leave a summary (optional)" bind:value={form.body}></textarea>
      <div class="buttons">
        <button class="ctl" type="button" onclick={() => (form.open = false)}>Cancel</button>
        <button class="ctl primary" type="button" disabled={sending} onclick={() => void submit()}>Submit</button>
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
  .n {
    margin-left: 6px;
    font-family: var(--font-mono);
  }
  .link {
    color: var(--accent);
    font-size: var(--fs-ctl);
  }
  .form {
    flex: 1 0 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-ctl);
  }
  textarea {
    width: 100%;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
</style>

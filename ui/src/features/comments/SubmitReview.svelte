<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import { session } from "../../state/session.svelte";
  import { platform } from "../../state/platform.svelte";
  import { threads } from "../../state/threads.svelte";
  import { draftsToSubmit, submitBlock } from "./pullRequest";

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
    if (!done) return;
    [form.open, form.body] = [false, ""];
    session.notice =
      done.skipped > 0
        ? `Review submitted. ${done.skipped} ${done.skipped === 1 ? "draft waits" : "drafts wait"}: its code is gone or its thread has an edit to settle.`
        : "Review submitted";
  }
</script>

<div class="submit">
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
  {#if blocked}
    <button class="flat link" type="button" onclick={() => platform.openAccounts()}>Sign in…</button>
  {/if}
  {#if form.open && !blocked}
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

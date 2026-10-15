<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import { verdictId, verdictLabel, verdictOf } from "../../lib/verdicts";
  import { platform } from "../../state/platform.svelte";
  import { commentsUi } from "./commentsUi.svelte";
  import { chosenVerdict } from "./posting";

  /** What goes with the viewer's comments: the review's verdict and summary, or the vote where it is set apart (staged, and sent with the comments). */
  let {
    info,
    part,
    disabled = false,
  }: { info: PullRequestInfo; part: "review" | "vote"; disabled?: boolean } = $props();

  // The form's state is the store's, so the Comments tool window and the toolbar's popover share it.
  const form = commentsUi.review;
  const shown = $derived(info.pendingVote?.verdict ?? info.viewerVerdict);
  const chosen = $derived(chosenVerdict(info));
  const value = $derived(chosen === undefined ? "" : verdictId(chosen));
</script>

{#if part === "review" && info.traits.batchedReview}
  <label>
    <span>Verdict</span>
    <select class="field" {value} onchange={(e) => (form.verdict = verdictOf(e.currentTarget.value))}>
      {#each info.traits.verdicts as v (v.type)}<option value={verdictId(v)}>{verdictLabel(v)}</option>{/each}
    </select>
  </label>
  <textarea class="field" rows="3" placeholder="Leave a summary (optional)" bind:value={form.body}></textarea>
{:else if part === "vote" && info.traits.verdictSeparate}
  <div class="vote">
    <label>
      <span>Vote</span>
      <select
        class="field"
        disabled={disabled || info.pendingVote?.state.type === "posting"}
        value={shown ? verdictId(shown) : ""}
        onchange={(e) => {
          const verdict = verdictOf(e.currentTarget.value);
          if (verdict) void platform.stageVote(verdict);
        }}
      >
        {#if !shown}<option value="" disabled>No vote</option>{/if}
        {#each info.traits.verdicts as v (v.type)}<option value={verdictId(v)}>{verdictLabel(v)}</option>{/each}
      </select>
    </label>
  </div>
{/if}

<style>
  .vote {
    display: flex;
    align-items: center;
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
    box-sizing: border-box;
  }
</style>

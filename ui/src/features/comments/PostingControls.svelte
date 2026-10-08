<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import { platform } from "../../state/platform.svelte";
  import { chosenVerdict } from "./posting";

  /** What goes with the viewer's comments: the review's verdict and summary, or the vote where it is set apart. */
  let {
    info,
    part,
    disabled = false,
  }: { info: PullRequestInfo; part: "review" | "vote"; disabled?: boolean } = $props();

  // The form's state is the store's, so the Comments tool window and the toolbar's popover share it.
  const form = platform.review;
</script>

{#if part === "review" && info.traits.batchedReview}
  <label>
    <span>Verdict</span>
    <select class="field" value={chosenVerdict(info)} onchange={(e) => (form.verdict = e.currentTarget.value)}>
      {#each info.traits.verdicts as v (v.id)}<option value={v.id}>{v.label}</option>{/each}
    </select>
  </label>
  <textarea class="field" rows="3" placeholder="Leave a summary (optional)" bind:value={form.body}></textarea>
{:else if part === "vote" && info.traits.verdictSeparate}
  <label>
    <span>Vote</span>
    <select
      class="field"
      {disabled}
      value={info.viewerVerdict ?? ""}
      onchange={(e) => void platform.vote(e.currentTarget.value)}
    >
      {#if !info.viewerVerdict}<option value="" disabled>No vote</option>{/if}
      {#each info.traits.verdicts as v (v.id)}<option value={v.id}>{v.label}</option>{/each}
    </select>
  </label>
{/if}

<style>
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

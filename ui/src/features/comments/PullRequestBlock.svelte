<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import type { ReviewerState } from "../../generated/ReviewerState";
  import { blockerText, reviewerMark, stateLabel } from "./pullRequest";

  let { info }: { info: PullRequestInfo } = $props();

  const verdictClass = (state: ReviewerState) => `verdict-${state}`;
</script>

<div class="pr">
  <div class="line">
    <span class="state state-{info.state}">{stateLabel(info)}</span>
    <span class="who">{info.author}</span>
    <span class="mono branches" title="{info.sourceBranch} into {info.targetBranch}">
      {info.sourceBranch} → {info.targetBranch}
    </span>
  </div>
  {#if info.reviewers.length}
    <ul class="reviewers" aria-label="Reviewers">
      {#each info.reviewers as reviewer (reviewer.name)}
        <li class={verdictClass(reviewer.state)} title={reviewer.state}>
          <span class="mark" aria-hidden="true">{reviewerMark(reviewer.state)}</span>{reviewer.name}
        </li>
      {/each}
    </ul>
  {/if}
  {#if info.labels.length}
    <ul class="labels" aria-label="Labels">
      {#each info.labels as label (label)}<li class="badge">{label}</li>{/each}
    </ul>
  {/if}
  {#if info.ci}<div class="ci ci-{info.ci}">Checks: {info.ci}</div>{/if}
  {#if info.blocking.length}
    <div class="blocking" role="status">Blocking: {info.blocking.map(blockerText).join(", ")}</div>
  {/if}
</div>

<style>
  .pr {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--line-soft);
    font-size: var(--fs-ctl);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .state {
    padding: 0 6px;
    border-radius: 8px;
    background: var(--ok);
    color: var(--on-primary);
    font-size: 11px;
  }
  .state-closed {
    background: var(--warn);
  }
  .state-merged {
    background: var(--note);
  }
  .who {
    font-weight: 600;
  }
  .branches {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
  }
  .mark {
    margin-right: 3px;
  }
  .verdict-approved .mark {
    color: var(--ok);
  }
  .verdict-changesRequested .mark {
    color: var(--warn);
  }
  .verdict-waiting,
  .verdict-commented {
    color: var(--text-2);
  }
  .ci-failure {
    color: var(--warn);
  }
  .ci-success {
    color: var(--ok);
  }
  .blocking {
    color: var(--warn);
    font-weight: 600;
  }
</style>

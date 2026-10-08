<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import Avatar from "../../lib/ui/Avatar.svelte";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import { blockerText, reviewerMark, stateLabel } from "./pullRequest";

  let { info }: { info: PullRequestInfo } = $props();
</script>

<dl class="pr">
  <dt>State</dt>
  <dd><span class="dot state-{info.state}"></span>{stateLabel(info)}</dd>
  <dt>Author</dt>
  <dd class="author"><Avatar name={info.author} url={info.authorAvatar} />{info.author}</dd>
  <dt>Branch</dt>
  <dd class="mono branches" title="{info.sourceBranch} into {info.targetBranch}">
    {info.sourceBranch} → {info.targetBranch}
  </dd>
  {#if info.reviewers.length}
    <dt>Reviewers</dt>
    <dd class="list">
      {#each info.reviewers as reviewer (reviewer.name)}
        <span class="verdict-{reviewer.state}" title={reviewer.state}>
          <span aria-hidden="true">{reviewerMark(reviewer.state)}</span>
          {reviewer.name}
        </span>
      {/each}
    </dd>
  {/if}
  {#if info.labels.length}
    <dt>Labels</dt>
    <dd class="list">
      {#each info.labels as label (label)}<span class="badge">{label}</span>{/each}
    </dd>
  {/if}
  {#if info.ci}
    <dt>Checks</dt>
    <dd><CiStatus state={info.ci} /></dd>
  {/if}
  {#if info.blocking.length}
    <dt>Blocking</dt>
    <dd class="blocking">{info.blocking.map(blockerText).join(" · ")}</dd>
  {/if}
</dl>

<style>
  .pr {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    margin: 0;
    padding: 8px 10px;
    border-bottom: 1px solid var(--line-soft);
    font-size: var(--fs-ctl);
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .author {
    gap: 6px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
  }
  .dot.state-closed {
    background: var(--warn);
  }
  .dot.state-merged {
    background: var(--note);
  }
  .branches {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-mono-s);
  }
  .list {
    flex-wrap: wrap;
    gap: 4px 10px;
  }
  .verdict-approved span {
    color: var(--ok);
  }
  .verdict-changesRequested span {
    color: var(--warn);
  }
  .blocking {
    color: var(--warn);
  }
</style>

<script lang="ts">
  import type { PullRequestInfo } from "../../generated/PullRequestInfo";
  import Avatar from "../../lib/ui/Avatar.svelte";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import Facts from "../../lib/ui/Facts.svelte";
  import StatusDot from "../../lib/ui/StatusDot.svelte";
  import { blockerText, reviewerMark, stateLabel, stateTone } from "./pullRequest";

  let { info }: { info: PullRequestInfo } = $props();
</script>

<div class="pr">
  <Facts>
    <dt>State</dt>
    <dd><StatusDot tone={stateTone(info.state)} />{stateLabel(info)}</dd>
    <dt>Author</dt>
    <dd class="author">
      <Avatar name={info.authorName} url={info.authorAvatar} /><span class="cut" title={info.authorName}
        >{info.authorName}</span
      >
    </dd>
    <dt>Branch</dt>
    <dd class="mono branches" title="{info.sourceBranch} into {info.targetBranch}">
      {info.sourceBranch} → {info.targetBranch}
    </dd>
    {#if info.reviewers.length}
      <dt>Reviewers</dt>
      <dd class="list">
        {#each info.reviewers as reviewer (reviewer.login)}
          <span class="reviewer verdict-{reviewer.state}" title="{reviewer.name}: {reviewer.state}">
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
      <dd class="blocking">
        {#each info.blocking as blocker, i (i)}<span class="item">{blockerText(blocker)}</span>{/each}
      </dd>
    {/if}
  </Facts>
</div>

<style>
  .pr {
    padding: 8px 10px;
    border-bottom: 1px solid var(--line-soft);
    font-size: var(--fs-ctl);
  }
  dd {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .cut,
  .reviewer {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    flex-wrap: wrap;
    gap: 0 6px;
    color: var(--warn);
  }
  /* The items wrap between each other, never inside one. */
  .item {
    white-space: nowrap;
  }
  .item + .item::before {
    content: "· ";
  }
</style>

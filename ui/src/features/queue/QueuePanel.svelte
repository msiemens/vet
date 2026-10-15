<script lang="ts">
  // The side panel of the queue: what the changeset on view is, why it is blocked, where the viewer's
  // review stands, its comments, checks and reviewers. It is for reading: the review is where to act.
  import type { Snippet } from "svelte";
  import { untrack } from "svelte";
  import type { CheckRun } from "../../generated/CheckRun";
  import type { Thread } from "../../generated/Thread";
  import { flagPreference } from "../../lib/preferences.svelte";
  import { platformName } from "../../lib/platformName";
  import Avatar from "../../lib/ui/Avatar.svelte";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import GroupLabel from "../../lib/ui/GroupLabel.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import SectionHead from "../../lib/ui/SectionHead.svelte";
  import SeenMeter from "../../lib/ui/SeenMeter.svelte";
  import { CheckRuns, failedFirst, passed } from "../checks";
  import { ThreadCards, groupThreads, threadPlace } from "../comments";
  import { markdownBody, needsShowAll, buildBriefCard } from "../review";
  import type { Frame } from "../../state/reviewModel";
  import { browse } from "../../state/browse";
  import { changeset } from "../../state/changeset.svelte";
  import { checks } from "../../state/checks.svelte";
  import { commands } from "../../state/commands";
  import { minute } from "../../state/minute.svelte";
  import { platform } from "../../state/platform.svelte";
  import { reviewChanges } from "../../state/reviewChanges.svelte";
  import { threads } from "../../state/threads.svelte";
  import ChangesetName from "./ChangesetName.svelte";
  import { metaOf, reasonOf, reviewSummary, reviewerLabel, reviewersSummary } from "./queuePanel";
  import { type QueueRow, labelOf, statusTitle } from "./queueRows";
  import StatusChip from "./StatusChip.svelte";

  let {
    row,
    frame,
    onopenthread,
    onopenlog,
    onbasechanged,
  }: {
    /** The selected changeset, which is the one on view; none selects nothing. */
    row?: QueueRow;
    /** The numbers of the changeset on view as a whole. */
    frame?: Frame;
    /** Shows a thread in the review. */
    onopenthread: (thread: Thread) => void;
    /** Shows the log of a failed run in the review. */
    onopenlog: (run: CheckRun) => void;
    /** Shows the review with the Changes tab filtered to the lines seen against an older base. */
    onbasechanged: () => void;
  } = $props();

  // Whether each section is open, remembered; every one starts open.
  const sections = new Map(
    ["description", "review", "comments", "checks", "reviewers"].map((id) => [
      id,
      flagPreference("app", `queue.section.${id}`, true),
    ]),
  );
  const isShut = (id: string) => sections.get(id)?.value === false;
  function fold(id: string) {
    const section = sections.get(id);
    if (section) section.value = !section.value;
  }

  // Whether resolved threads show open or as one row: one setting for every changeset.
  const resolvedOpen = flagPreference("app", "queue.resolvedOpen", false);
  function flipResolved() {
    resolvedOpen.value = !resolvedOpen.value;
  }

  const pull = $derived(row?.pullRequest);
  const info = $derived(platform.info);
  const headId = $derived(
    changeset.facts?.target.changeset.type === "pullRequest" ? changeset.facts.target.head : undefined,
  );
  const head = $derived(checks.of(headId));
  const kind = $derived(platform.status?.kind ?? info?.platform.kind);
  const groups = $derived(groupThreads(threads.items));
  const openThreads = $derived([...groups.conflicts, ...groups.open]);
  const reason = $derived(
    pull &&
      reasonOf({
        pull,
        info,
        platform: kind,
        failed: (head?.runs ?? []).filter((r) => r.state === "failure").map((r) => r.name),
        unread: info?.unread.replies ?? 0,
      }),
  );
  /** The version the review last marked; what changed since it is asked of the backend (D95). */
  const reviewed = $derived(row?.review.state === "open" ? row.review.lastReviewed : undefined);
  const changesSince = $derived(reviewChanges.of(reviewed));
  $effect(() => {
    const version = reviewed;
    untrack(() => reviewChanges.ensure(version));
  });
  const summary = $derived(
    row &&
      reviewSummary({
        review: row.review,
        completion: frame?.facts.completion,
        files: frame?.facts.files,
        asked: pull?.waitingForMe ?? false,
        base: frame && { branch: pull?.targetBranch, shortId: frame.layout.base.commit.shortId },
        changesSince,
        noun: row.key.type === "pullRequest" ? ["push", "pushes"] : ["version", "versions"],
      }),
  );
  const description = $derived(info?.description.trim() ?? "");
  let descriptionAll = $state(false);
  const reviewers = $derived(info?.reviewers ?? []);

  function openInBrowser() {
    if (row) commands.run("changeset.openInBrowser", commands.context({ row: { key: row.key, url: pull?.url } }));
  }

  const card = (thread: Thread, isOpen: boolean, toggle: () => void) =>
    buildBriefCard(thread, isOpen, {
      place: threadPlace(thread),
      toggle,
      openThread: () => onopenthread(thread),
      openLink: browse,
    });
</script>

{#snippet section(id: string, title: string, count: number | undefined, body: Snippet, folded?: Snippet)}
  <section>
    <SectionHead
      {title}
      {count}
      open={!isShut(id)}
      ontoggle={() => fold(id)}
      summary={isShut(id) ? folded : undefined}
    />
    {#if !isShut(id)}{@render body()}{/if}
  </section>
{/snippet}

<aside aria-label="Selected changeset">
  <div class="sh sh-bar">
    <span>Changeset</span>
    <span class="grow"></span>
    <IconButton
      icon="external"
      label="Open in the browser"
      size="s"
      iconSize={13}
      disabled={pull?.url === undefined}
      onclick={openInBrowser}
    />
  </div>
  {#if row && summary}
    <ScrollArea>
      <div>
        <div class="head">
          <div class="title"><ChangesetName label={labelOf(row)} /></div>
          <div class="meta">
            {#each metaOf(row, frame?.facts.files, minute.now) as item, i (i)}
              {#if i > 0}<span class="sep">·</span>{/if}
              {#if item.type === "author"}
                {#if !item.muted}<Avatar name={item.name} url={item.avatar ?? null} />{/if}
                <span class:muted={item.muted}>{item.name}</span>
              {:else if item.type === "mono"}
                <span class="mono branches">{item.text}</span>
              {:else}
                <span>{item.text}</span>
              {/if}
            {/each}
          </div>
          {#if pull}
            <div class="reason">
              <span class="chipbox"><StatusChip status={pull.status} title={statusTitle(pull)} /></span>
              {#if reason}
                <span class="why" class:bad={reason.tone === "bad"}>
                  {#each reason.parts as part, i (i)}
                    <span class="part"
                      >{#if part.bold}<b>{part.bold}</b>{/if}{part.text}</span
                    >
                  {/each}
                  {#if reason.protection}
                    <span class="part">
                      Blocked by branch protection ·
                      <button class="link" type="button" onclick={openInBrowser}
                        >Open on {platformName(kind ?? "github")} <Icon name="external" size={11} /></button
                      >
                    </span>
                  {/if}
                </span>
              {/if}
            </div>
          {/if}
        </div>

        {#if description}
          {#snippet descriptionBody()}
            <div class="body">
              <div
                class:vet-msg-clamp={!descriptionAll && needsShowAll(description)}
                {@attach (box: HTMLElement) => {
                  box.replaceChildren(markdownBody(description, { openLink: browse }));
                }}
              ></div>
              {#if !descriptionAll && needsShowAll(description)}
                <button class="link small" type="button" onclick={() => (descriptionAll = true)}>Show all</button>
              {/if}
            </div>
          {/snippet}
          {@render section("description", "Description", undefined, descriptionBody)}
        {/if}

        {#snippet reviewBody()}
          <div class="body">
            <div class="line">
              <span
                ><b>{summary.state}</b>{#if summary.detail}<span class="muted detail">{summary.detail}</span>{/if}</span
              >
              <span class="grow"></span>
              {#if summary.percent !== undefined}<span class="mono pct">{summary.percent} %</span>{/if}
            </div>
            <SeenMeter seen={summary.seen} old={summary.oldBase} />
            {#if summary.asked}
              <div class="line top muted">
                <span class="accent"><Icon name="bell" size={12} /></span><span>Your review is asked for.</span>
              </div>
            {/if}
            {#if summary.base}
              <div class="line top muted"><Icon name="branch" size={12} /><span>{summary.base}</span></div>
            {/if}
            {#if summary.pushes}
              <div class="line top muted"><span class="strong">{summary.pushes}</span></div>
            {/if}
            {#if summary.oldBaseNote}
              <div class="line top">
                <span class="stripes"></span>
                <button class="link wrap" type="button" onclick={onbasechanged}>{summary.oldBaseNote}</button>
              </div>
            {/if}
          </div>
        {/snippet}
        {@render section("review", "Your review", undefined, reviewBody)}

        {#snippet commentsBody()}
          {#if openThreads.length}
            <GroupLabel count={openThreads.length}>Open</GroupLabel>
            <div class="cards"><ThreadCards list={openThreads} {card} /></div>
          {:else}
            <EmptyState align="start" text={groups.resolved.length ? "No open threads." : "No comments yet."} />
          {/if}
          {#if groups.resolved.length}
            <GroupLabel count={groups.resolved.length}>Resolved</GroupLabel>
            <div class="cards">
              <ThreadCards list={groups.resolved} {card} isOpen={() => resolvedOpen.value} ontoggle={flipResolved} />
            </div>
          {/if}
        {/snippet}
        {@render section("comments", "Comments", threads.items.length, commentsBody)}

        {#if head?.runs.length}
          {#snippet checksBody()}
            <div class="runs"><CheckRuns runs={failedFirst(head.runs)} onlog={onopenlog} markFailed /></div>
          {/snippet}
          {#snippet foldedChecks()}
            <CiStatus state={head.state} size={12} />{passed(head).passed} passed
          {/snippet}
          {@render section("checks", "Checks", head.runs.length, checksBody, foldedChecks)}
        {/if}

        {#if reviewers.length}
          {#snippet reviewersBody()}
            <div class="body">
              <div class="kv">
                {#each reviewers as reviewer (reviewer.login)}
                  <Avatar name={reviewer.name} url={null} />
                  <span class="name">{reviewer.name}</span>
                  <span class="state {reviewer.state}">{reviewerLabel(reviewer.state)}</span>
                {/each}
              </div>
            </div>
          {/snippet}
          {#snippet foldedReviewers()}{reviewersSummary(reviewers)}{/snippet}
          {@render section("reviewers", "Reviewers", reviewers.length, reviewersBody, foldedReviewers)}
        {/if}
      </div>
    </ScrollArea>
  {:else}
    <EmptyState title="Select a changeset" text="Its status, your progress and its comments show here." />
  {/if}
</aside>

<style>
  aside {
    flex: none;
    width: var(--w-queue-panel);
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-left: 1px solid var(--line);
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
  }
  .title {
    font-size: var(--fs);
    font-weight: 600;
    line-height: 1.35;
    text-wrap: pretty;
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
    font-size: var(--fs-ctl);
    color: var(--text-2);
  }
  .branches {
    font-size: var(--fs-mono-s);
  }
  .sep {
    color: var(--text-3);
  }
  .muted {
    color: var(--text-2);
  }
  .reason {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: var(--fs-ctl);
  }
  .chipbox {
    flex: none;
  }
  .why {
    color: var(--text-2);
  }
  .why.bad {
    color: var(--del-text);
  }
  .part + .part::before,
  .detail::before {
    content: "·";
    margin: 0 0.4em;
  }
  .part b {
    font-weight: 600;
  }
  .link.wrap {
    white-space: normal;
    text-align: left;
  }
  .small {
    font-size: var(--fs-mono-s);
  }
  .body {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    padding: 2px 12px 10px var(--inset-tree-name);
    font-size: var(--fs-ctl);
    line-height: 1.5;
  }
  .body > .link {
    align-self: flex-start;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .line.top {
    align-items: flex-start;
  }
  .line.top > :global(svg),
  .line.top > .accent {
    margin-top: 3px;
  }
  .accent {
    display: inline-flex;
    color: var(--accent);
  }
  .strong {
    color: var(--text);
    font-weight: 600;
  }
  .pct {
    font-size: var(--fs-mono);
  }
  .stripes {
    flex: none;
    width: 12px;
    height: 4px;
    margin-top: 8px;
    border-radius: 2px;
    background: var(--old-base-stripes);
  }
  /* The runs start under the section's title, not under its chevron. */
  .runs {
    padding-left: var(--w-chev);
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 2px 10px 10px;
  }
  .kv {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    column-gap: 8px;
    row-gap: 4px;
    align-items: center;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .state.approved {
    color: var(--ok);
  }
  .state.changesRequested {
    color: var(--warn);
  }
</style>

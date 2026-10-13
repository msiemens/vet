<script module lang="ts">
  export type Screen = "queue" | "review";
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Completion } from "../../generated/Completion";
  import type { ReviewInfo } from "../../generated/ReviewInfo";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import SeenMeter from "../../lib/ui/SeenMeter.svelte";
  import { commands } from "../../state/commands";
  import BaseNote from "./BaseNote.svelte";
  import { completionTitle, nothingToReview } from "./completionTitle";
  import ScopeChip from "./ScopeChip.svelte";
  import type { BaseNote as BaseNoteInfo, ScopeChip as ScopeInfo } from "./scopeText";
  import TitleBar from "./TitleBar.svelte";

  let {
    screen,
    project,
    changesets,
    baseNote,
    scope,
    onscope,
    onwhole,
    review,
    completion,
    refreshing = false,
    filtered,
    checks,
    outgoing,
  }: {
    /** Which screen shows: the queue of changesets, or the review of the one on view. */
    screen: Screen;
    /** The project's name with its switcher. */
    project: Snippet;
    /** The changeset's name with its switcher; shown in the review. */
    changesets: Snippet;
    /** What the changes are measured from, where that is worth saying. */
    baseNote?: BaseNoteInfo;
    /** What the review view shows. */
    scope?: ScopeInfo;
    /** Opens or focuses the timeline; none while there are no versions. */
    onscope?: () => void;
    /** Goes back to all changes. */
    onwhole: () => void;
    /** Whether a review has started; Finish Review shows only while it is open. */
    review?: ReviewInfo;
    /** The progress of what the review view shows. */
    completion?: Completion;
    /** The queue is being refreshed. */
    refreshing?: boolean;
    /** While the review view shows a filtered part of the changeset: what acts on it and the way back, before the progress. */
    filtered?: Snippet;
    /** The head commit's checks, after the Finish Review button. */
    checks?: Snippet;
    /** The viewer's comments on their way to the platform, at the end. */
    outgoing?: Snippet;
  } = $props();
  const SCREENS = [
    { id: "queue", icon: "queue" },
    { id: "review", icon: "review" },
  ] as const;
  const seen = $derived(completion?.seenPercent ?? 0);
  const oldBase = $derived(completion?.oldBasePercent ?? 0);
  const nothing = $derived(completion !== undefined && nothingToReview(completion));
  const title = $derived(completion && completionTitle(completion));
</script>

<TitleBar>
  <Segmented
    label="Screen"
    options={SCREENS.map(({ id, icon }) => ({
      id,
      icon,
      label: commands.label(`view.${id}`),
      title: commands.hint(`view.${id}`),
      disabled: !commands.isEnabled(`view.${id}`),
    }))}
    value={screen}
    onchange={(id) => commands.run(`view.${id}`)}
  />
  {@render project()}
  {#if screen === "review"}
    <span class="range" data-tauri-drag-region>
      {@render changesets()}
      {#if baseNote}<BaseNote note={baseNote} />{/if}
      {#if scope}<ScopeChip chip={scope} onopen={onscope} onclear={onwhole} />{/if}
    </span>
  {/if}
  <span class="grow" data-tauri-drag-region></span>
  {#if screen === "queue"}
    <IconButton
      icon="reconfirm"
      label={refreshing ? "Fetching…" : commands.label("changeset.refresh")}
      title={commands.hint("changeset.refresh")}
      disabled={!commands.isEnabled("changeset.refresh")}
      onclick={() => commands.run("changeset.refresh")}
    />
  {:else}
    {@render filtered?.()}
    <span class="completion" {title} data-tauri-drag-region>
      <SeenMeter width={96} {seen} old={oldBase} pct={!nothing} />
      {#if nothing}<span class="none">nothing to review</span>{/if}
    </span>
    {#if review?.state === "open"}
      <button
        class="ctl finish"
        type="button"
        title={commands.hint("review.finish", "End this review; edits are browsed until the next mark")}
        disabled={!commands.isEnabled("review.finish")}
        onclick={() => commands.run("review.finish")}>{commands.label("review.finish")}</button
      >
    {/if}
    {@render checks?.()}
    {@render outgoing?.()}
  {/if}
  <IconButton icon="search" label="Search everywhere" onclick={() => commands.run("palette.all")} />
</TitleBar>

<style>
  .range {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .finish {
    margin-right: 6px;
  }
  .grow {
    flex: 1 1 40px;
  }
  .completion {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin-right: 6px;
  }
  .none {
    font-size: var(--fs-ctl);
    color: var(--text-3);
  }
</style>

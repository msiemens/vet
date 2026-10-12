<script module lang="ts">
  export type Screen = "queue" | "review";
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Base } from "../../generated/Base";
  import type { BaseChoice } from "../../generated/BaseChoice";
  import type { Completion } from "../../generated/Completion";
  import type { ReviewInfo } from "../../generated/ReviewInfo";
  import type { Target } from "../../generated/Target";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import type { BaseCandidates } from "../../state/backend";
  import { commands } from "../../state/commands";
  import BaseChip from "./BaseChip.svelte";
  import { completionTitle } from "./completionTitle";
  import TitleBar from "./TitleBar.svelte";
  import VersionLabel from "./VersionLabel.svelte";

  let {
    screen,
    project,
    changesets,
    base,
    target,
    review,
    loadBase,
    onbase,
    completion,
    refreshing = false,
    chips,
    back,
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
    /** What the changes are compared against. */
    base?: Base;
    /** What they lead to: the working tree, or a branch's tip. */
    target?: Target;
    /** Whether a review has started; its version shows only while it is open. */
    review?: ReviewInfo;
    /** Lists the branches and commits the base can be picked from. */
    loadBase: () => Promise<BaseCandidates | undefined>;
    onbase: (choice: BaseChoice) => void;
    /** The changeset's progress. */
    completion?: Completion;
    /** The queue is being refreshed. */
    refreshing?: boolean;
    /** While the review view shows a range of versions: its ends, in place of the base and the progress. */
    chips?: Snippet;
    /** The way back to the changeset, at the right end. */
    back?: Snippet;
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
      {#if chips}
        {@render chips()}
      {:else}
        <BaseChip
          {base}
          branch={target?.branch}
          own={target?.changeset.type === "branch" ? target.name : undefined}
          load={loadBase}
          onpick={onbase}
        />
        <span class="arrow" data-tauri-drag-region><Icon name="arrow" stroke={1.8} /></span>
        {#if review?.state === "open"}
          <VersionLabel version={review.version} updatedAt={review.updatedAt} />
        {:else if target}
          <span class="target mono" data-tauri-drag-region
            >{target.changeset.type === "branch" && target.head ? target.head.slice(0, 7) : target.name}</span
          >
        {/if}
        {#if target?.changeset.type === "workingTree"}<span
            class="live-dot"
            aria-label="Follows the working tree"
            data-tauri-drag-region
          ></span>{/if}
      {/if}
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
  {:else if chips}
    {@render back?.()}
  {:else}
    {@render filtered?.()}
    <span class="completion" {title} data-tauri-drag-region>
      <Meter
        width={96}
        segments={[
          { value: seen, color: "var(--seen)" },
          { value: oldBase, color: "var(--old-base-stripes)" },
        ]}
      />
      <span class="mono pct">{Math.round(seen)}%</span>
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
  <IconButton icon="search" label="Search everywhere" />
</TitleBar>

<style>
  .range {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .arrow {
    display: inline-flex;
    color: var(--text-3);
  }
  .live-dot {
    margin-left: 0;
  }
  .target {
    padding: 2px 6px;
    color: var(--text-2);
    font-size: var(--fs-mono);
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
  .pct {
    font-size: var(--fs-mono);
  }
</style>

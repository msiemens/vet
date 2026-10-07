<script lang="ts">
  import type { Base } from "../../generated/Base";
  import type { BaseChoice } from "../../generated/BaseChoice";
  import type { Completion } from "../../generated/Completion";
  import type { ReviewInfo } from "../../generated/ReviewInfo";
  import type { Target } from "../../generated/Target";
  import Chip from "../../lib/ui/Chip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import type { BaseCandidates } from "../../state/backend";
  import { openItem } from "../../state/mainMenu";
  import { mainMenu } from "../../state/mainMenu.svelte";
  import BaseChip from "./BaseChip.svelte";
  import TitleBar from "./TitleBar.svelte";
  import VersionLabel from "./VersionLabel.svelte";

  let {
    project,
    base,
    target,
    review,
    loadBase,
    onbase,
    completion,
    onfinish,
  }: {
    project: string;
    /** What the changes are compared against. */
    base?: Base;
    /** What they lead to: the working tree. */
    target?: Target;
    /** Whether a review has started; its version shows only while it is open. */
    review?: ReviewInfo;
    /** Lists the branches and commits the base can be picked from. */
    loadBase: () => Promise<BaseCandidates | undefined>;
    onbase: (choice: BaseChoice) => void;
    /** "Finish review", offered while a review is open. */
    onfinish: () => void;
    /** The changeset's progress. */
    completion?: Completion;
  } = $props();
  let projectMenu = $state(false);
  const seen = $derived(completion?.seenPercent ?? 0);
  const oldBase = $derived(completion?.oldBasePercent ?? 0);
  const title = $derived(
    completion &&
      `${completion.seenLines} of ${completion.lines} lines · ${completion.acknowledgedItems} of ${completion.items} other changes · ${completion.oldBaseLines} lines against an old base`,
  );
</script>

<TitleBar>
  <Popover bind:open={projectMenu}>
    {#snippet trigger(toggle)}
      <Chip bold onclick={toggle}>{project}</Chip>
    {/snippet}
    <Menu
      items={[openItem]}
      onselect={(id) => {
        projectMenu = false;
        mainMenu.run(id);
      }}
    />
  </Popover>
  <span class="range" data-tauri-drag-region>
    <BaseChip {base} branch={target?.branch} load={loadBase} onpick={onbase} />
    <span class="arrow" data-tauri-drag-region><Icon name="arrow" stroke={1.8} /></span>
    <span class="target" title={target?.branch && `On ${target.branch}`} data-tauri-drag-region
      >{target?.name ?? "…"}</span
    >
    {#if review?.state === "open"}
      <VersionLabel version={review.version} updatedAt={review.updatedAt} />
    {/if}
  </span>
  <span class="grow" data-tauri-drag-region></span>
  <span class="completion" {title} data-tauri-drag-region>
    <Meter
      width={96}
      segments={[
        { value: seen, color: "var(--seen)" },
        { value: oldBase, color: "var(--old-base)" },
      ]}
    />
    <span class="mono pct">{Math.round(seen + oldBase)}%</span>
  </span>
  {#if review?.state === "open"}
    <button
      class="ctl finish"
      type="button"
      title="End this review; edits are browsed until the next mark"
      onclick={onfinish}>Finish review</button
    >
  {/if}
  <IconButton icon="search" label="Search everywhere" />
</TitleBar>

<style>
  .range {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: 4px;
  }
  .arrow {
    display: inline-flex;
    color: var(--text-3);
  }
  .target {
    padding: 0 4px;
    color: var(--text-2);
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

<script module lang="ts">
  export interface HeaderInfo {
    project: string;
    /** "Working tree", a PR, a commit range. */
    changeset: { label: string; branch: string };
    target: { label: string; settled: boolean };
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import type { Base } from "../../generated/Base";
  import type { BaseChoice } from "../../generated/BaseChoice";
  import Chip from "../../lib/ui/Chip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import type { BaseCandidates } from "../../state/backend";
  import BaseChip from "./BaseChip.svelte";
  import TitleBar from "./TitleBar.svelte";

  let {
    info,
    base,
    loadBase,
    onbase,
    completion,
    filter,
    run,
    onopenrepo,
  }: {
    info: HeaderInfo;
    /** What the changes are compared against. */
    base?: Base;
    /** Lists the branches and commits the base can be picked from. */
    loadBase: () => Promise<BaseCandidates | undefined>;
    onbase: (choice: BaseChoice) => void;
    /** "Open repository…" in the project menu. */
    onopenrepo: () => void;
    /** Share of changed lines seen and seen against an old base, in percent. */
    completion: { seenPct: number; oldPct: number; title?: string };
    /** The filter button with its popover. */
    filter?: Snippet;
    /** The Run split button. */
    run?: Snippet;
  } = $props();
  let projectMenu = $state(false);
</script>

<TitleBar>
  <Popover bind:open={projectMenu}>
    {#snippet trigger(toggle)}
      <Chip bold onclick={toggle}>{info.project}</Chip>
    {/snippet}
    <Menu
      items={[{ id: "open", label: "Open repository…" }]}
      onselect={() => {
        projectMenu = false;
        onopenrepo();
      }}
    />
  </Popover>
  <Chip>
    <Icon name="branch" />{info.changeset.label}<span class="dim">{info.changeset.branch}</span><Icon
      name="caret"
      size={10}
      stroke={2}
    />
  </Chip>
  <span class="range" data-tauri-drag-region>
    <BaseChip {base} load={loadBase} onpick={onbase} />
    <span class="arrow" data-tauri-drag-region><Icon name="arrow" stroke={1.8} /></span>
    <Chip mono
      >{info.target.label}{#if info.target.settled}<span class="dot"></span>{/if}</Chip
    >
  </span>
  {#if filter}{@render filter()}{/if}
  <span class="grow" data-tauri-drag-region></span>
  <span class="completion" title={completion.title} data-tauri-drag-region>
    <Meter
      width={96}
      segments={[
        { value: completion.seenPct, color: "var(--seen)" },
        { value: completion.oldPct, color: "var(--old-base)" },
      ]}
    />
    <span class="mono pct">{Math.round(completion.seenPct + completion.oldPct)}%</span>
  </span>
  {#if run}{@render run()}{/if}
  <IconButton icon="search" label="Search everywhere" />
</TitleBar>

<style>
  .dim {
    color: var(--text-2);
  }
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
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
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

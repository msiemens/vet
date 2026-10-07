<script lang="ts">
  import type { VersionInfo } from "../../generated/VersionInfo";
  import Chip from "../../lib/ui/Chip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import type { MenuItem } from "../../lib/ui/menuItem";
  import Popover from "../../lib/ui/Popover.svelte";
  import { type Range, withFrom, withTo } from "../../state/timelineRange";

  let {
    range,
    versions,
    onrange,
  }: {
    /** The versions compared; the dot on its end says it follows the working tree. */
    range: Range;
    /** Oldest first. */
    versions: readonly VersionInfo[];
    onrange: (range: Range) => void;
  } = $props();

  let fromOpen = $state(false);
  let toOpen = $state(false);

  const latest = $derived(versions.at(-1)?.n ?? range.to);
  const time = (at: number) => new Date(at * 1000).toLocaleTimeString(undefined, { hour12: false });

  /** The versions that can be an end: those before `range.to` for the start, after `range.from` for the end. */
  const items = (end: "from" | "to"): MenuItem[] =>
    versions
      .filter((v) => (end === "from" ? v.n < range.to : v.n > range.from))
      .map((v) => ({ id: String(v.n), label: `v${v.n}`, detail: time(v.at), checked: v.n === range[end] }))
      .reverse();
</script>

<Popover bind:open={fromOpen}>
  {#snippet trigger(toggle)}
    <Chip mono active={fromOpen} title="Where the range starts" onclick={toggle}>v{range.from}</Chip>
  {/snippet}
  <Menu
    items={items("from")}
    onselect={(id) => {
      fromOpen = false;
      onrange(withFrom(range, Number(id)));
    }}
  />
</Popover>
<span class="arrow"><Icon name="arrow" stroke={1.8} /></span>
<Popover bind:open={toOpen}>
  {#snippet trigger(toggle)}
    <Chip
      mono
      active={toOpen}
      title={range.live ? "Where the range ends; it follows the working tree" : "Where the range ends"}
      onclick={toggle}
    >
      v{range.to}{#if range.live}<span class="live-dot" aria-label="Follows the working tree"></span>{/if}
    </Chip>
  {/snippet}
  <Menu
    items={items("to")}
    onselect={(id) => {
      toOpen = false;
      onrange(withTo(range, Number(id), latest));
    }}
  />
</Popover>

<style>
  .arrow {
    display: inline-flex;
    color: var(--text-3);
  }
</style>

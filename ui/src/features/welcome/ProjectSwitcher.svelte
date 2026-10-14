<script lang="ts">
  import type { RecentProject } from "../../generated/RecentProject";
  import Chip from "../../lib/ui/Chip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import SwitcherPopover from "../../lib/ui/SwitcherPopover.svelte";
  import { commands } from "../../state/commands";
  import { openRecent, openRecentMenu } from "./openRecent";
  import RecentRow from "./RecentRow.svelte";
  import { type RecentMatch, searchRecent } from "./recentList";

  let {
    name,
    path,
    recent,
    host,
    canPick,
  }: {
    /** The open project's name and path. */
    name: string;
    path: string;
    /** Backend-side paths with when they were opened, most recent first. */
    recent: RecentProject[];
    /** Where the projects live (the backend's host), if it is worth saying. */
    host?: string;
    /** A native folder dialog exists. */
    canPick: boolean;
  } = $props();

  let open = $state(false);
  let query = $state("");

  const found = $derived(searchRecent(recent, query));
</script>

<SwitcherPopover
  bind:open
  bind:query
  label="Search projects"
  items={found}
  key={(m: RecentMatch) => m.project.path}
  isOn={(m) => m.project.path === path}
  listLabel="Recent projects"
  empty={recent.length ? "No matching projects" : "No recent projects"}
  actions={[
    ...(canPick
      ? [
          {
            label: "Open…",
            key: commands.binding("file.open")?.label,
            title: commands.hint("file.open"),
            disabled: !commands.isEnabled("file.open"),
            run: () => commands.run("file.open"),
          },
        ]
      : []),
    {
      label: "Project Settings…",
      icon: "settings",
      title: commands.hint("app.projectSettings"),
      disabled: !commands.isEnabled("app.projectSettings"),
      run: () => commands.run("app.projectSettings"),
    },
  ]}
  onchoose={(m) => {
    if (m.project.path !== path) openRecent(m.project.path);
  }}
  onmenu={(m, e) => openRecentMenu(e, m.project.path)}
>
  {#snippet trigger(toggle)}
    <Chip bold active={open} onclick={toggle}>{name}</Chip>
  {/snippet}
  {#snippet row(m)}
    <RecentRow match={m}>
      {#snippet trailing()}
        {#if host}<span class="host">{host}</span>{/if}
        <span class="check"
          >{#if m.project.path === path}<Icon name="check" size={13} stroke={2.2} />{/if}</span
        >
      {/snippet}
    </RecentRow>
  {/snippet}
</SwitcherPopover>

<style>
  .host {
    flex: none;
    font-size: var(--fs-mono-s);
    color: var(--text-2);
  }
  .check {
    flex: none;
    width: var(--s-glyph);
    display: inline-flex;
    color: var(--accent);
  }
</style>

<script module lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  /** The areas the viewer folded; kept while the window lives. */
  const folded = new SvelteSet<string>();
</script>

<script lang="ts">
  import type { ContextMenuRequest } from "../../lib/contextMenu";
  import { bindingOf, specOf } from "../../lib/keymap";
  import { conflictsOf } from "../../lib/keymapEdit";
  import type { Column } from "../../lib/ui/columns";
  import ContextMenu from "../../lib/ui/ContextMenu.svelte";
  import Kbd from "../../lib/ui/Kbd.svelte";
  import { ListNav, revealCurrent } from "../../lib/ui/listNav";
  import type { MenuItem } from "../../lib/ui/menuItem";
  import SearchField from "../../lib/ui/SearchField.svelte";
  import TreeRow from "../../lib/ui/TreeRow.svelte";
  import { commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { isMac } from "../../state/host";
  import { keymap } from "../../state/keymap.svelte";
  import SettingsTable from "./SettingsTable.svelte";
  import { type KeymapEntry, type KeymapRow, keymapEntries, keymapGroups, namesText } from "./keymapRows";

  const COLUMNS: Column<never>[] = [
    { id: "command", label: "Command", flex: true, min: 120 },
    { id: "binding", label: "Binding", width: 250, min: 120 },
  ];

  let query = $state("");
  /** The command whose new key is being read from the keyboard. */
  let recording = $state<string>();
  /** A key that another command has: waits for the viewer to take it over or give it up. */
  let clash = $state<{ id: string; spec: string; key: string; others: string[] }>();
  let problem = $state<string>();
  let highlighted = $state(-1);
  let body = $state<HTMLElement>();
  /** The row menu, drawn here: the dialog is modal, so the window's own menu would lie behind it. */
  let menu = $state.raw<ContextMenuRequest>();

  const listing = $derived(commands.list());
  const groups = $derived(keymapGroups(listing, keymap.overrides, query));
  const entries = $derived(keymapEntries(groups, folded, query.trim() !== ""));
  const anyChanged = $derived(Object.keys(keymap.overrides).length > 0);

  // While a key is being read, every key goes here and runs nothing.
  $effect(() => {
    const id = recording;
    if (id === undefined) return;
    return commands.capture((e) => {
      if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
        recording = undefined;
        return;
      }
      const spec = specOf(e);
      if (spec === null) {
        if (!["Control", "Alt", "Shift", "Meta"].includes(e.key)) {
          problem = "That key combination types a character or cannot be a shortcut. Press another one.";
        }
        return;
      }
      recording = undefined;
      const others = conflictsOf(listing, id, spec);
      if (others.length === 0) keymap.bind(id, spec);
      else
        clash = { id, spec, key: bindingOf(spec, isMac ? "Meta" : "Ctrl").label, others: others.map((c) => c.label) };
    });
  });

  function record(id: string) {
    clash = undefined;
    problem = undefined;
    recording = id;
  }

  function toggle(area: string) {
    if (!folded.delete(area)) folded.add(area);
  }

  const nav = new ListNav<KeymapEntry>({
    element: () => body,
    items: () => entries,
    single: (entry) => entry.kind === "group" && toggle(entry.area),
    double: (entry) => (entry.kind === "group" ? toggle(entry.area) : record(entry.row.id)),
    highlight: { get: () => highlighted, set: (i) => (highlighted = i) },
    reveal: () => void revealCurrent(body),
  });
  $effect(() => () => nav.cancel());

  function menuOf(row: KeymapRow): MenuItem[] {
    return [
      { id: "change", label: "Change" },
      { id: "remove", label: "Remove", enabled: row.binding !== undefined },
      { id: "reset", label: "Reset", enabled: row.changed },
    ];
  }

  /** A right click highlights the row and opens its menu. */
  function openMenu(e: MouseEvent, row: KeymapRow, index: number) {
    nav.cancel();
    highlighted = index;
    void contextMenus.show(
      {
        x: e.clientX,
        y: e.clientY,
        items: menuOf(row),
        onselect: (choice) => {
          if (choice === "change") record(row.id);
          else if (choice === "remove") keymap.unbind(row.id);
          else keymap.reset(row.id);
        },
      },
      (request) => (menu = request),
    );
  }

  function onkeydown(e: KeyboardEvent) {
    if (recording !== undefined || !nav.keydown(e)) return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<!-- Escape ends the menu, not the dialog. -->
<svelte:window
  onpointerdown={(e) => {
    if (recording !== undefined && !(e.target as Element).closest(`[data-command="${recording}"]`))
      recording = undefined;
  }}
  onkeydown={(e) => {
    if (menu && e.key === "Escape") e.preventDefault();
  }}
/>

{#if menu}
  <ContextMenu {...menu} onclose={() => (menu = undefined)} />
{/if}

<div class="page">
  <div class="bar">
    <SearchField bind:value={query} label="Search commands and keys" placeholder="Search commands and keys" />
    <button class="ctl" type="button" disabled={!anyChanged} onclick={() => keymap.resetAll()}>Reset all</button>
  </div>

  {#if problem}<p class="settings-problem">{problem}</p>{/if}

  <SettingsTable
    name="keymap"
    columns={COLUMNS}
    gap="4px"
    row="auto"
    bind:body
    role="listbox"
    aria-label="Commands"
    {onkeydown}
    {...nav.attributes}
  >
    {#each entries as entry, index (entry.kind === "group" ? `group:${entry.area}` : entry.row.id)}
      {#if entry.kind === "group"}
        <div class="group">
          <TreeRow
            open={entry.open}
            heading
            letters={false}
            count={entry.count}
            countBesideName
            cur={index === highlighted}
            tabindex={-1}
            onclick={(e) => nav.click(e, index)}
          >
            {entry.label}
          </TreeRow>
        </div>
      {:else}
        {@const row = entry.row}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="flat pick row entry"
          class:on={index === highlighted}
          role="option"
          tabindex="-1"
          aria-selected={index === highlighted}
          data-command={row.id}
          onclick={(e) => nav.click(e, index)}
          oncontextmenu={(e) => {
            e.preventDefault();
            openMenu(e, row, index);
          }}
        >
          <span class="cell name">{row.label}</span>
          <span class="cell">
            {#if recording === row.id}
              <span class="settings-hint">Press a key combination, Esc to cancel</span>
            {:else if row.binding}
              <Kbd keys={row.binding.label} accent={row.changed} />
              {#each row.alsoBindings as also (also.spec)}
                <Kbd keys={also.label} />
              {/each}
            {:else}
              <span class="none">{row.changed ? "Removed" : "None"}</span>
            {/if}
          </span>
          {#if row.sharedWith.length > 0 || clash?.id === row.id}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="sub" onclick={(e) => e.stopPropagation()} ondblclick={(e) => e.stopPropagation()}>
              {#if row.sharedWith.length}
                <span class="settings-problem">Also used by {namesText(row.sharedWith)}</span>
              {/if}
              {#if clash?.id === row.id}
                <span class="settings-problem">
                  <Kbd keys={clash.key} /> is already used by {namesText(clash.others)}
                </span>
                <button
                  class="ctl small"
                  type="button"
                  onclick={() => {
                    if (clash) keymap.bind(clash.id, clash.spec);
                    clash = undefined;
                  }}>Take it over</button
                >
                <button class="ctl small" type="button" onclick={() => (clash = undefined)}>Keep as is</button>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    {:else}
      <p class="empty">No command matches.</p>
    {/each}
  </SettingsTable>
</div>

<style>
  .page {
    font-size: var(--fs-ctl);
  }
  p {
    margin: 0;
  }
  .bar {
    display: flex;
    gap: 8px;
    margin-bottom: 12px;
  }
  .group {
    border-bottom: 1px solid var(--line-soft);
    background: var(--bg-panel);
  }
  .group :global(.tw) {
    height: var(--h-ctl);
  }
  .row.entry {
    align-items: start;
    user-select: none;
    -webkit-user-select: none;
  }
  .entry:not(.on):hover {
    background: var(--hover);
  }
  .cell {
    min-height: 29px;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .name {
    padding-left: var(--inset-tree-name);
  }
  .sub {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    padding: 0 0 6px var(--inset-tree-name);
  }
  .none {
    color: var(--text-3);
    font-size: var(--fs-mono);
  }
  .empty {
    padding: 12px 10px;
    color: var(--text-3);
  }
</style>

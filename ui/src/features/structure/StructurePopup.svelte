<script lang="ts">
  // The structure of the active file in a popup over the editor: the same list as the window, with
  // a filter field on top. A click or Enter jumps to the symbol and closes it; Escape closes it.
  import type { SymbolInfo } from "../../generated/SymbolInfo";
  import { LIST } from "../../lib/listKeys";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import KeyHint from "../../lib/ui/KeyHint.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import PopupHeader from "../../lib/ui/PopupHeader.svelte";
  import PopupShell from "../../lib/ui/PopupShell.svelte";
  import TextToggle from "../../lib/ui/TextToggle.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import { structure } from "../../state/structure.svelte";
  import { structureRows, type StructureRow } from "../../state/structureRows";
  import { keyOf } from "../../state/symbolKey";
  import StructureList from "./StructureList.svelte";
  import StructureNotice from "./StructureNotice.svelte";

  let {
    onjump,
    oneditor,
  }: {
    /** Shows the symbol in the file's tab, at the base side's text for a removed one. */
    onjump: (symbol: SymbolInfo, removed: boolean) => void;
    /** Gives the keyboard focus back to the editor. */
    oneditor: () => void;
  } = $props();

  let query = $state("");
  let cursor = $state(0);
  let list = $state<VirtualRows>();
  let keepFocus = $state(false);

  const target = $derived(structure.target);
  const outline = $derived(structure.load.state === "done" ? structure.load.outline : undefined);
  const symbols = $derived(outline?.symbols ?? []);
  const marked = $derived(symbols.some((s) => s.change !== null));
  const rows = $derived(
    outline && !outline.unavailable
      ? structureRows(symbols, {
          sort: structure.sort,
          changedOnly: structure.changedOnly && marked,
          query,
          closed: new Set(),
        })
      : [],
  );
  const fileName = $derived(target?.path.split("/").pop());

  // The popup opens on the symbol the cursor is in; what is typed or toggled starts the list over.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the reads make them dependencies
    [query, structure.sort, structure.changedOnly];
    cursor = Math.max(0, query ? rows.findIndex((r) => !r.muted) : rows.findIndex(selected));
  });

  const selected = (r: StructureRow) => keyOf(r.symbol.id) === structure.selected;

  function close() {
    structure.popup = false;
  }

  function jump(row: StructureRow | undefined) {
    if (!row) return;
    keepFocus = true;
    close();
    onjump(row.symbol, row.change === "removed");
  }

  const nav = new ListNav<StructureRow>({
    items: () => rows,
    single: jump,
    highlight: { get: () => cursor, set: (i) => (cursor = i) },
    reveal: (i) => void list?.reveal(i),
    field: true,
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !e.isComposing) {
      e.preventDefault();
      e.stopPropagation();
      close();
      oneditor();
      return;
    }
    if (e.key === "F4" && !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      jump(rows.at(cursor));
      return;
    }
    if (nav.keydown(e)) {
      e.preventDefault();
      e.stopPropagation();
    }
  }
</script>

{#snippet head()}
  <PopupHeader>
    Structure of <code>{fileName}</code>
    {#snippet meta()}
      {#if outline && !outline.unavailable}<span class="n">{symbols.length}</span>{/if}
    {/snippet}
    {#snippet controls()}
      <TextToggle on={structure.sort} title="Sort alphabetically" onclick={() => structure.setSort(!structure.sort)}
        >A–Z</TextToggle
      >
      <IconButton
        icon="filter"
        label={marked ? "Changed only" : "Changed only (nothing is changed in this file)"}
        active={structure.changedOnly && marked}
        disabled={!marked}
        size={22}
        iconSize={14}
        onclick={() => structure.setChangedOnly(!structure.changedOnly)}
      />
    {/snippet}
  </PopupHeader>
{/snippet}

{#snippet foot()}
  <KeyHint keys="↑↓">move</KeyHint>
  <KeyHint keys="↵">jump</KeyHint>
  <KeyHint keys="esc">close</KeyHint>
{/snippet}

<PopupShell
  label="Structure of {fileName ?? 'the file'}"
  place="position:absolute;z-index:20;top:44px;left:50%;transform:translateX(-50%);width:420px;max-width:calc(100% - 16px);max-height:440px"
  placeholder="Filter symbols"
  scroll={false}
  attributes={{ [LIST]: "" }}
  bind:value={query}
  bind:keepFocus
  onclose={close}
  {onkeydown}
  {head}
  {foot}
>
  {#if rows.length}
    <StructureList
      bind:list
      {rows}
      selected={undefined}
      {cursor}
      onclick={(_, i) => jump(rows.at(i))}
      onhover={(e, i) => nav.hover(e, i)}
    />
  {:else}
    <StructureNotice load={structure.load} file={fileName} matches={rows.length} />
  {/if}
</PopupShell>

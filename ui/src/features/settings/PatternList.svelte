<!-- A list of globs of the review filter. Each row says what the backend counted for the pattern in the
     changeset on view, or why it is invalid; the add row says so for the text typed into it. -->
<script lang="ts">
  import type { PatternMatch } from "../../generated/PatternMatch";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import type { Column } from "../../lib/ui/columns";
  import SettingsTable from "./SettingsTable.svelte";
  import { type PatternDraft, matchText, withEdit } from "./filterDraft";

  let {
    name,
    label,
    patterns,
    draft = $bindable({ edits: {}, adding: "" }),
    matches,
    onchange,
  }: {
    /** Keeps the column widths under this name. */
    name: string;
    label: string;
    patterns: readonly string[];
    /** What is typed and not saved yet. */
    draft?: PatternDraft;
    /** For each pattern as drafted: the rows' edits applied, then the add field's text. */
    matches: readonly PatternMatch[] | undefined;
    /** Saves the patterns; resolves with the backend's message if it refused them. */
    onchange: (patterns: string[]) => Promise<string | undefined>;
  } = $props();

  const COLUMNS: Column<never>[] = [
    { id: "pattern", label: "Pattern", flex: true, min: 80 },
    { id: "matches", label: "Matches", width: 260, min: 100 },
    { id: "actions", label: "", width: 56, fixed: true },
  ];

  const added = $derived(draft.adding.trim());
  const invalid = (i: number) => matches?.[i]?.invalid ?? undefined;

  async function commit(index: number) {
    const text = draft.edits[index];
    if (text === undefined) return;
    if (text.trim() !== "" && invalid(index) !== undefined) return;
    if (text.trim() === patterns[index] || (await onchange(withEdit(patterns, index, text))) === undefined) {
      draft.edits = {};
    }
  }

  async function remove(index: number) {
    if ((await onchange(patterns.filter((_, j) => j !== index))) === undefined) draft.edits = {};
  }

  async function add() {
    if (!added || invalid(patterns.length) !== undefined || patterns.includes(added)) return;
    if ((await onchange([...patterns, added])) === undefined) draft.adding = "";
  }
</script>

{#snippet counted(index: number)}
  {@const match = matches?.[index]}
  {#if match?.invalid}
    <span class="err clip" title={match.invalid}>{match.invalid}</span>
  {:else if match}
    <span class="clip muted">{matchText(match)}</span>
  {:else}
    <span></span>
  {/if}
{/snippet}

<SettingsTable {name} aria-label={label} columns={COLUMNS}>
  {#each patterns as pattern, i (i)}
    <div class="row">
      <input
        class="field mono"
        class:bad={invalid(i) !== undefined}
        aria-label="{label} pattern"
        spellcheck="false"
        value={draft.edits[i] ?? pattern}
        oninput={(e) => (draft.edits[i] = e.currentTarget.value)}
        onchange={() => void commit(i)}
        onkeydown={(e) => {
          if (e.key === "Enter") void commit(i);
        }}
      />
      {@render counted(i)}
      <span class="end">
        <IconButton icon="close" label="Remove {pattern}" size="s" iconSize={12} onclick={() => void remove(i)} />
      </span>
    </div>
  {/each}
  {#snippet addRow()}
    <div class="row">
      <input
        class="field mono"
        class:bad={added !== "" && invalid(patterns.length) !== undefined}
        aria-label="Add {label} pattern"
        spellcheck="false"
        bind:value={draft.adding}
        onkeydown={(e) => {
          if (e.key === "Enter") void add();
        }}
      />
      {#if added}{@render counted(patterns.length)}{:else}<span></span>{/if}
      <span class="end">
        <button
          class="ctl"
          type="button"
          disabled={!added || invalid(patterns.length) !== undefined || patterns.includes(added)}
          onclick={() => void add()}>Add</button
        >
      </span>
    </div>
  {/snippet}
</SettingsTable>

<style>
  .clip {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .muted {
    color: var(--text-2);
  }
  .bad {
    border-color: var(--del-text);
  }
  .end {
    display: flex;
    justify-content: flex-end;
  }
</style>

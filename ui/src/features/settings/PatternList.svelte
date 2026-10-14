<!-- A list of globs of the review filter. Each row says what the backend counted for the pattern in the
     changeset on view, or why it is invalid; the add row says so for the text typed into it. -->
<script lang="ts">
  import type { PatternMatch } from "../../generated/PatternMatch";
  import FilterList from "./FilterList.svelte";
  import { type PatternDraft, matchText, withEdit } from "./filterDraft";

  let {
    label,
    placeholder,
    patterns,
    draft = $bindable({ edits: {}, adding: "" }),
    matches,
    onchange,
  }: {
    label: string;
    placeholder: string;
    patterns: readonly string[];
    /** What is typed and not saved yet. */
    draft?: PatternDraft;
    /** For each pattern as drafted: the rows' edits applied, then the add field's text. */
    matches: readonly PatternMatch[] | undefined;
    /** Saves the patterns; resolves with the backend's message if it refused them. */
    onchange: (patterns: string[]) => Promise<string | undefined>;
  } = $props();

  const added = $derived(draft.adding.trim());
  const invalid = (i: number) => matches?.[i]?.invalid ?? undefined;
  const counted = (i: number) => {
    const match = matches?.[i];
    return match?.invalid ?? (match ? matchText(match) : undefined);
  };
  const rows = $derived(
    patterns.map((pattern, i) => ({
      text: draft.edits[i] ?? pattern,
      meta: counted(i),
      bad: invalid(i) !== undefined,
    })),
  );
  const addBad = $derived(added !== "" && invalid(patterns.length) !== undefined);

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

<FilterList
  {label}
  {rows}
  {placeholder}
  bind:adding={draft.adding}
  addMeta={added ? counted(patterns.length) : undefined}
  {addBad}
  canAdd={added !== "" && invalid(patterns.length) === undefined && !patterns.includes(added)}
  oninput={(i, text) => {
    draft.edits[i] = text;
  }}
  oncommit={(i) => void commit(i)}
  onremove={(i) => void remove(i)}
  onadd={() => void add()}
/>

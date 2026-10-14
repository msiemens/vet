<script module lang="ts">
  export interface FilterRow {
    text: string;
    /** What the backend counted for the entry, on the right of its row. */
    meta?: string;
    /** The entry is invalid: `meta` says why. */
    bad?: boolean;
  }
</script>

<script lang="ts">
  // A list of the review filter: the entries (the roots, or the globs of one rule), each with what
  // the backend counted for it and a button to remove it, and under them the strip for adding one.
  import type { Snippet } from "svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";

  let {
    label,
    rows,
    glyph,
    placeholder,
    adding = $bindable(""),
    previews = false,
    addMeta,
    addBad = false,
    canAdd,
    problem,
    extra,
    oninput,
    oncommit,
    onremove,
    onadd,
    onaddinput,
  }: {
    /** What assistive technology calls the list; the add field and the remove buttons build on it. */
    label: string;
    rows: readonly FilterRow[];
    /** Before the text of every row. */
    glyph?: Snippet<[text: string]>;
    placeholder: string;
    /** The text of the add field. */
    adding?: string;
    /** The add field has room beside it for what the backend counted for its text. */
    previews?: boolean;
    /** What the backend counted for the text in the add field. */
    addMeta?: string;
    /** The add field holds an invalid entry. */
    addBad?: boolean;
    canAdd: boolean;
    /** What is wrong with the text in the add field, below the strip. */
    problem?: string;
    /** In the strip, between the field and Add. */
    extra?: Snippet;
    /** The rows' text is editable: called with the row and the text typed into it. */
    oninput?: (index: number, text: string) => void;
    /** An edited row is committed. */
    oncommit?: (index: number) => void;
    onremove: (index: number) => void;
    onadd: () => void;
    onaddinput?: () => void;
  } = $props();
</script>

{#snippet meta(text: string | undefined, bad: boolean | undefined)}
  <span class="meta" class:err={bad}>{text ?? ""}</span>
{/snippet}

<div class="box" role="group" aria-label={label}>
  {#each rows as row, i (i)}
    <div class="row">
      {@render glyph?.(row.text)}
      {#if oninput}
        <input
          class="field mono"
          class:bad={row.bad}
          aria-label="{label} entry"
          spellcheck="false"
          value={row.text}
          oninput={(e) => oninput(i, e.currentTarget.value)}
          onchange={() => oncommit?.(i)}
          onkeydown={(e) => {
            if (e.key === "Enter") oncommit?.(i);
          }}
        />
      {:else}
        <span class="path mono clip" title={row.text}>{row.text}</span>
      {/if}
      {@render meta(row.meta, row.bad)}
      <IconButton icon="close" label="Remove {row.text}" size="s" iconSize={12} onclick={() => onremove(i)} />
    </div>
  {/each}
  <div class="row add">
    <input
      class="field mono"
      class:bad={addBad}
      aria-label="Add to {label}"
      {placeholder}
      spellcheck="false"
      bind:value={adding}
      oninput={onaddinput}
      onkeydown={(e) => {
        if (e.key === "Enter" && canAdd) onadd();
      }}
    />
    {#if previews}{@render meta(addMeta, addBad)}{/if}
    {@render extra?.()}
    <button class="ctl" type="button" disabled={!canAdd} onclick={onadd}>Add</button>
  </div>
  {#if problem}<p class="err problem">{problem}</p>{/if}
</div>

<style>
  .box {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-editor);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    box-sizing: border-box;
    min-height: 32px;
    padding: 4px 6px 4px 8px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-3);
  }
  .row.add {
    background: var(--bg-panel);
    border-radius: 0 0 var(--radius) var(--radius);
  }
  .row:last-child {
    border-bottom: 0;
  }
  .row > .field,
  .path {
    flex: 1;
    min-width: 0;
  }
  .path {
    font-size: var(--fs-mono);
    color: var(--text);
  }
  .row:not(.add) > .field {
    border-color: transparent;
    background: transparent;
  }
  .row:not(.add) > .field:hover {
    border-color: var(--line);
  }
  .row:not(.add) > .field:focus {
    border-color: var(--focus);
    background: var(--bg-editor);
  }
  .field.bad,
  .row:not(.add) > .field.bad {
    border-color: var(--del-text);
  }
  .meta {
    flex: 0 1 auto;
    min-width: 150px;
    max-width: 50%;
    text-align: right;
    font-size: var(--fs-mono);
    color: var(--text-2);
  }
  .meta.err {
    color: var(--del-text);
  }
  .clip {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .problem {
    margin: 0;
    padding: 4px 10px;
    border-top: 1px solid var(--line-soft);
    font-size: var(--fs-mono);
  }
</style>

<!-- The review roots: a list with a field to add a folder by its path, checked against the repository's
     folders, or from a tree of them. -->
<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { changeset } from "../../state/changeset.svelte";
  import FolderPicker from "./FolderPicker.svelte";
  import SettingsTable from "./SettingsTable.svelte";
  import { rootPath } from "./filterDraft";

  let {
    roots,
    onchange,
  }: {
    roots: readonly string[];
    /** Saves the roots; resolves with the backend's message if it refused them. */
    onchange: (roots: string[]) => Promise<string | undefined>;
  } = $props();

  const COLUMNS: Column<never>[] = [
    { id: "folder", label: "Folder", flex: true, min: 80 },
    { id: "actions", label: "", width: 150, fixed: true },
  ];

  let text = $state("");
  let problem = $state<string>();
  let picking = $state(false);
  let asked = 0;

  async function problemOf(path: string): Promise<string | undefined> {
    if (roots.includes(path)) return "Already a review root.";
    return (await changeset.folders(path)) ? undefined : `There is no folder "${path}" in this repository.`;
  }

  /** Checks the typed path as it is typed; an answer to an older text is dropped. */
  async function typed() {
    const path = rootPath(text);
    const n = ++asked;
    const found = path ? await problemOf(path) : undefined;
    if (n === asked) problem = found;
  }

  async function add(path: string) {
    const refused = (await problemOf(path)) ?? (await onchange([...roots, path]));
    problem = refused;
    if (refused) return;
    text = "";
    picking = false;
  }
</script>

<SettingsTable name="roots" aria-label="Review roots" columns={COLUMNS}>
  {#each roots as root, i (root)}
    <div class="row">
      <span class="path" title={root}>{root}</span>
      <span class="end">
        <IconButton
          icon="close"
          label="Remove {root}"
          size="s"
          iconSize={12}
          onclick={() => void onchange(roots.filter((_, j) => j !== i))}
        />
      </span>
    </div>
  {/each}
  {#snippet addRow()}
    <div class="row">
      <input
        class="field mono"
        aria-label="Add review root"
        spellcheck="false"
        bind:value={text}
        oninput={() => void typed()}
        onkeydown={(e) => {
          if (e.key === "Enter" && rootPath(text)) void add(rootPath(text));
        }}
      />
      <span class="end">
        <button class="ctl" type="button" aria-expanded={picking} onclick={() => (picking = !picking)}>Browse…</button>
        <button
          class="ctl"
          type="button"
          disabled={!rootPath(text) || problem !== undefined}
          onclick={() => void add(rootPath(text))}>Add</button
        >
      </span>
    </div>
  {/snippet}
  {#snippet below()}
    {#if problem}<p class="err problem">{problem}</p>{/if}
    {#if picking}<FolderPicker onpick={(path) => void add(path)} oncancel={() => (picking = false)} />{/if}
  {/snippet}
</SettingsTable>

<style>
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .end {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .problem {
    margin: 0;
    padding: 4px 10px;
    border-top: 1px solid var(--line-soft);
    font-size: var(--fs-mono);
  }
</style>

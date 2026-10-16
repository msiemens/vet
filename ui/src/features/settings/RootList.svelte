<!-- The review roots: a list with a field to add a folder by its path, checked against the repository's
     folders, or from a popover with a tree of them. -->
<script lang="ts">
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { folders } from "../../state/projectRequests";
  import { Latest } from "../../state/resource";
  import FilterList from "./FilterList.svelte";
  import FolderPicker from "./FolderPicker.svelte";
  import { rootPath, rootText } from "./filterDraft";

  let {
    roots,
    counts,
    onchange,
  }: {
    roots: readonly string[];
    /** The changed files below each root, as the backend counted them. */
    counts: ReadonlyMap<string, number> | undefined;
    /** Saves the roots; resolves with the backend's message if it refused them. */
    onchange: (roots: string[]) => Promise<string | undefined>;
  } = $props();

  let text = $state("");
  let problem = $state<string>();
  let picking = $state(false);
  const asked = new Latest();

  const path = $derived(rootPath(text));
  const rows = $derived(
    roots.map((root) => {
      const files = counts?.get(root);
      return { text: root, meta: files === undefined ? undefined : rootText(files) };
    }),
  );

  async function problemOf(path: string): Promise<string | undefined> {
    if (roots.includes(path)) return "Already a review root.";
    return (await folders(path)) ? undefined : `There is no folder "${path}" in this repository.`;
  }

  /** Checks the typed path as it is typed; an answer to an older text is dropped. */
  async function typed() {
    const isLatest = asked.begin();
    const found = path ? await problemOf(path) : undefined;
    if (isLatest()) problem = found;
  }

  async function add(path: string) {
    const refused = (await problemOf(path)) ?? (await onchange([...roots, path]));
    problem = refused;
    if (refused) return;
    text = "";
    picking = false;
  }
</script>

<FilterList
  label="Review roots"
  {rows}
  placeholder="Type a folder, e.g. crates/vet-app"
  bind:adding={text}
  addBad={problem !== undefined}
  canAdd={path !== "" && problem === undefined}
  {problem}
  onaddinput={() => void typed()}
  onremove={(i) => void onchange(roots.filter((_, j) => j !== i))}
  onadd={() => void add(path)}
>
  {#snippet glyph(root)}<FileIcon name={root} folder />{/snippet}
  {#snippet extra()}
    <Popover bind:open={picking} align="right">
      {#snippet trigger(toggle)}
        <button class="ctl" type="button" aria-expanded={picking} onclick={toggle}>Browse…</button>
      {/snippet}
      <FolderPicker onpick={(picked) => void add(picked)} oncancel={() => (picking = false)} />
    </Popover>
  {/snippet}
</FilterList>

<script lang="ts">
  import type { Base } from "../../generated/Base";
  import type { BaseChoice } from "../../generated/BaseChoice";
  import Chip from "../../lib/ui/Chip.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import type { BaseCandidates } from "../../state/backend";
  import { behindText } from "../../state/baseBehind";
  import { baseMenuItems, baseTooltip, parseBaseItem } from "./baseText";
  import CommitPicker from "./CommitPicker.svelte";

  let {
    base,
    branch,
    own,
    load,
    onpick,
  }: {
    /** What the changeset is compared against now; unknown until the manifest arrives. */
    base?: Base;
    /** The working tree's branch. */
    branch?: string;
    /** The branch a branch changeset is of; it is not offered as its own target. */
    own?: string;
    /** Branches and recent commits; `undefined` if they could not be listed. */
    load: () => Promise<BaseCandidates | undefined>;
    onpick: (choice: BaseChoice) => void;
  } = $props();

  let open = $state(false);
  let view = $state<"menu" | "commits">("menu");
  let candidates = $state.raw<BaseCandidates>();

  // Candidates are listed afresh each time the menu opens.
  $effect(() => {
    if (!open) return;
    view = "menu";
    candidates = undefined;
    void load().then((c) => (candidates = c));
  });

  const items = $derived(baseMenuItems(candidates?.branches ?? [], base?.choice, !!candidates, behindText(base), own));

  function select(id: string) {
    const picked = parseBaseItem(id);
    if (picked === "commits") {
      view = "commits";
    } else if (picked) {
      open = false;
      onpick(picked);
    }
  }

  function pickCommit(id: string) {
    open = false;
    onpick({ type: "commit", id });
  }
</script>

<Popover bind:open>
  {#snippet trigger(toggle)}
    <Chip mono active={open} title={base ? baseTooltip(base, branch) : undefined} onclick={toggle}>
      {base?.name ?? "…"}
    </Chip>
  {/snippet}
  {#if view === "commits" && candidates}
    <CommitPicker commits={candidates.commits} current={base?.commit.id ?? ""} onpick={pickCommit} />
  {:else}
    <Menu speedSearch {items} onselect={select} />
  {/if}
</Popover>

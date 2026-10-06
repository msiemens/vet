<script lang="ts">
  import type { Base } from "../../generated/Base";
  import type { BaseChoice } from "../../generated/BaseChoice";
  import Chip from "../../lib/ui/Chip.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import type { MenuItem } from "../../lib/ui/menuItem";
  import Popover from "../../lib/ui/Popover.svelte";
  import type { BaseCandidates } from "../../state/backend";
  import CommitPicker from "./CommitPicker.svelte";

  let {
    base,
    load,
    onpick,
  }: {
    /** What the changeset is compared against now; unknown until the manifest arrives. */
    base?: Base;
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

  const label = $derived(base ? `${base.label} · ${base.commit.slice(0, 7)}` : "…");

  /** The default branch first, then the current one, then the rest as listed. */
  const branches = $derived(
    [...(candidates?.branches ?? [])].sort(
      (a, b) => Number(b.default) - Number(a.default) || Number(b.current) - Number(a.current),
    ),
  );

  const items = $derived.by((): MenuItem[] => {
    const choice = base?.choice;
    return [
      { id: "pin", label: "Session pin (HEAD at open)", checked: choice?.type === "pin" },
      {
        id: "mergeBase",
        label: "Merge-base with",
        enabled: branches.length > 0,
        checked: choice?.type === "mergeBase",
        children: branches.map((b) => ({
          id: `mb:${b.name}`,
          label: b.name,
          detail: b.default ? "default" : b.current ? "current" : undefined,
          checked: choice?.type === "mergeBase" && choice.reference === b.name,
        })),
      },
      { id: "commit", label: "Commit…", enabled: !!candidates, checked: choice?.type === "commit" },
    ];
  });

  function select(id: string) {
    if (id === "commit") {
      view = "commits";
      return;
    }
    open = false;
    if (id === "pin") onpick({ type: "pin" });
    else if (id.startsWith("mb:")) onpick({ type: "mergeBase", reference: id.slice(3) });
  }

  function pickCommit(id: string) {
    open = false;
    onpick({ type: "commit", id });
  }
</script>

<Popover bind:open>
  {#snippet trigger(toggle)}
    <Chip mono active={open} title="What the changes are compared against" onclick={toggle}>{label}</Chip>
  {/snippet}
  {#if view === "commits" && candidates}
    <CommitPicker commits={candidates.commits} current={base?.commit ?? ""} onpick={pickCommit} />
  {:else}
    <Menu {items} onselect={select} />
  {/if}
</Popover>

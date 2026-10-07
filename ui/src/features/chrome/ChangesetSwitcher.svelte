<script lang="ts">
  import type { ChangesetKey } from "../../generated/ChangesetKey";
  import type { ChangesetRow } from "../../generated/ChangesetRow";
  import Chip from "../../lib/ui/Chip.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { parseKeyId } from "../../state/changesetKey";
  import { changesetItems } from "./changesetMenu";

  let {
    name,
    title,
    load,
    onpick,
    open = $bindable(false),
  }: {
    /** What the changeset on view is called; unknown until the manifest arrives. */
    name?: string;
    title?: string;
    /** The changesets on offer and the one on view; `undefined` if they could not be listed. */
    load: () => Promise<{ rows: ChangesetRow[]; onView: ChangesetKey } | undefined>;
    onpick: (key: ChangesetKey) => void;
    open?: boolean;
  } = $props();

  let listed = $state.raw<{ rows: ChangesetRow[]; onView: ChangesetKey }>();

  // The changesets are listed afresh each time the menu opens.
  $effect(() => {
    if (!open) return;
    listed = undefined;
    void load().then((l) => (listed = l));
  });

  const items = $derived(listed ? changesetItems(listed.rows, listed.onView) : []);
</script>

<Popover bind:open>
  {#snippet trigger(toggle)}
    <Chip active={open} {title} onclick={toggle}>{name ?? "…"}</Chip>
  {/snippet}
  {#if listed}
    <Menu
      {items}
      onselect={(id) => {
        const key = parseKeyId(id);
        open = false;
        if (key) onpick(key);
      }}
    />
  {:else}
    <Menu items={[{ id: "loading", label: "Loading…", enabled: false }]} onselect={() => undefined} />
  {/if}
</Popover>

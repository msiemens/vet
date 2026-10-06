<script lang="ts">
  // A change card on its own, outside a section (the dev gallery). The card itself is DOM built by
  // `editor/changeCard.ts`, the same code the Changes stream uses.
  import type { EntryDesc } from "../../generated/EntryDesc";
  import type { NonTextKind } from "../../generated/NonTextKind";
  import { changeCard } from "./editor/changeCard";

  let {
    path,
    oldPath,
    kind,
    before,
    after,
    acknowledged,
    ontoggle,
  }: {
    path: string;
    oldPath: string | null;
    kind: NonTextKind;
    before: EntryDesc | null;
    after: EntryDesc | null;
    acknowledged: boolean;
    ontoggle: () => void;
  } = $props();
</script>

<div
  class="host"
  {@attach (host) => {
    const card = changeCard(path, oldPath, { kind, before, after }, acknowledged, {
      toggle: ontoggle,
      comment: () => undefined,
    });
    host.append(card.el);
    return () => card.el.remove();
  }}
></div>

<style>
  .host {
    border: 1px solid var(--line);
    border-radius: 6px;
    overflow: hidden;
  }
</style>

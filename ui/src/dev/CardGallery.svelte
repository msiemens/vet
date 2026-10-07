<script lang="ts">
  // Change cards on fixtures: the backend's corpus has no non-text items.
  import type { EntryDesc } from "../generated/EntryDesc";
  import type { NonTextItem } from "../generated/NonTextItem";
  import { ChangeCard } from "../features/review";

  const file = (mode: number, size: number | null = 12): EntryDesc => ({ kind: "file", mode, target: null, size });
  const cards: { path: string; oldPath?: string; item: NonTextItem }[] = [
    {
      path: "scripts/release.sh",
      item: {
        kind: "mode",
        before: file(0o100644),
        after: file(0o100755),
        hasPreview: false,
        modeChange: "gainedExecutable",
      },
    },
    {
      path: "assets/logo.png",
      item: {
        kind: "binary",
        before: file(0o100644, 4300),
        after: file(0o100644, 5900),
        hasPreview: true,
        modeChange: null,
      },
    },
    {
      path: "assets/current",
      item: {
        kind: "symlink",
        before: { kind: "symlink", mode: 0o120000, target: "releases/2024-05", size: null },
        after: { kind: "symlink", mode: 0o120000, target: "releases/2024-06", size: null },
        hasPreview: false,
        modeChange: null,
      },
    },
    {
      path: "vendor/zstd",
      item: {
        kind: "submodule",
        before: { kind: "submodule", mode: 0o160000, target: "a41c9e2f0", size: null },
        after: { kind: "submodule", mode: 0o160000, target: "0f3b7d1aa", size: null },
        hasPreview: false,
        modeChange: null,
      },
    },
    {
      path: "src/new_name.rs",
      oldPath: "src/old_name.rs",
      item: { kind: "rename", before: null, after: null, hasPreview: false, modeChange: null },
    },
    {
      path: "data/dump.bin",
      item: { kind: "contentUnavailable", before: null, after: null, hasPreview: false, modeChange: null },
    },
  ];
  let acknowledged = $state(cards.map(() => false));
  let { onclose }: { onclose: () => void } = $props();
</script>

<section class="gallery">
  <button class="ctl" type="button" onclick={onclose}>Close</button>
  <div class="cards">
    {#each cards as c, i (c.path)}
      <ChangeCard
        path={c.path}
        oldPath={c.oldPath ?? null}
        item={c.item}
        acknowledged={acknowledged[i]}
        ontoggle={() => (acknowledged[i] = !acknowledged[i])}
      />
    {/each}
  </div>
</section>

<style>
  .gallery {
    position: fixed;
    inset: 40px 20px 20px;
    overflow: auto;
    background: var(--bg-panel);
    border: 1px solid var(--line);
    padding: 10px;
    z-index: 30;
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 760px;
    margin-top: 10px;
  }
</style>

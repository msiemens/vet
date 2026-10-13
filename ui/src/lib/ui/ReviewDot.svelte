<script lang="ts">
  // What has been seen of the hunk a row is in, as the meters draw it: filled, striped against an old
  // base, hollow, or a dash outside any hunk.
  let { reviewed }: { reviewed: "seen" | "old" | "unseen" | "none" } = $props();

  const WORDS = {
    seen: "Seen",
    old: "Seen against an old base",
    unseen: "Not seen",
    none: "Outside any hunk",
  } as const;
</script>

<span class="rv {reviewed}" title={WORDS[reviewed]}><i></i></span>

<style>
  .rv {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
  }
  i {
    display: block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
  .seen i {
    background: var(--seen);
  }
  .old i {
    background: var(--old-base-stripes);
  }
  .unseen i {
    border: 1.5px solid var(--text-3);
    box-sizing: border-box;
  }
  .none i {
    width: 7px;
    height: 1.5px;
    border-radius: 1px;
    background: var(--text-3);
    opacity: 0.7;
  }
</style>

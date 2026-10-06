<script module lang="ts">
  import type { EntryDesc } from "../../generated/EntryDesc";
  import type { NonTextKind } from "../../generated/NonTextKind";

  const TITLE: Record<NonTextKind, string> = {
    mode: "File mode",
    symlink: "Symlink target",
    submodule: "Submodule commit",
    binary: "Binary file",
    contentUnavailable: "Content unavailable",
    rename: "Renamed",
  };
  const LABEL = { false: "Mark seen", true: "Mark unseen" } as const;
  const IMAGE = /\.(png|jpe?g|gif|webp|bmp|ico|svg)$/i;

  function size(bytes: number | null): string {
    if (bytes === null) return "";
    return bytes < 1024
      ? `${bytes} B`
      : bytes < 1048576
        ? `${(bytes / 1024).toFixed(1)} KB`
        : `${(bytes / 1048576).toFixed(1)} MB`;
  }

  /** One side of the change as text. */
  function describe(kind: NonTextKind, side: EntryDesc | null, path: string): string {
    if (!side) return "(none)";
    switch (kind) {
      case "mode":
        return side.mode.toString(8);
      case "symlink":
        return side.target ?? "";
      case "submodule":
        return side.target?.slice(0, 7) ?? "";
      case "binary":
        return size(side.size);
      case "rename":
        return path;
      default:
        return "";
    }
  }
</script>

<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";

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

  const state = $derived(acknowledged ? "seen" : "unseen");
  const label = $derived(LABEL[String(acknowledged) as "true" | "false"]);
</script>

<article class="card">
  <div class="vet-hunk vet-hunk-{state}">
    <span class="vet-hunk-text">{TITLE[kind]}</span>
    <span class="path mono">{path}</span>
    <span class="spacer"></span>
    <button class="vet-hunk-btn vet-seen-toggle" type="button" aria-label={label} title={label} onclick={ontoggle}
      ><Icon name="check" size={13} stroke={2} /></button
    >
    <button class="vet-hunk-btn" type="button" aria-label="Comment" title="Comment"
      ><Icon name="comments" size={13} stroke={1.8} /></button
    >
  </div>
  <div class="body">
    {#if kind === "contentUnavailable"}
      <span class="note"><Icon name="lock" size={14} />The content of this file is not available.</span>
    {:else}
      <div class="line del">
        <span class="sign">−</span><span class="mono"
          >{kind === "rename" ? (oldPath ?? "") : describe(kind, before, path)}</span
        >
      </div>
      <div class="line add"><span class="sign">+</span><span class="mono">{describe(kind, after, path)}</span></div>
      {#if kind === "binary" && IMAGE.test(path)}
        <div class="previews">
          {#each [["Before", before], ["After", after]] as [side, entry]}
            {#if entry}<figure>
                <div class="thumb"><Icon name="image" size={28} /></div>
                <figcaption>{side}</figcaption>
              </figure>{/if}
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</article>

<style>
  .card {
    border: 1px solid var(--ln);
    border-radius: 6px;
    overflow: hidden;
    background: var(--ed);
  }
  .path {
    font-size: var(--fs-mono-s);
    color: var(--tx);
  }
  .spacer {
    flex: 1;
  }
  .body {
    display: flex;
    flex-direction: column;
  }
  .line {
    display: flex;
    gap: 8px;
    height: var(--h-line);
    line-height: var(--h-line);
    padding: 0 10px;
    font-size: var(--fs-mono);
  }
  .sign {
    width: 14px;
    text-align: center;
  }
  .add {
    background: var(--add);
  }
  .add .sign {
    color: var(--addt);
  }
  .del {
    background: var(--del);
  }
  .del .sign {
    color: var(--delt);
  }
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    color: var(--tx2);
  }
  .previews {
    display: flex;
    gap: 16px;
    padding: 10px;
  }
  figure {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: center;
    color: var(--tx3);
    font-size: var(--fs-mono-s);
  }
  .thumb {
    width: 120px;
    height: 80px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px dashed var(--ln);
    border-radius: var(--radius);
    background: var(--panel);
  }
</style>

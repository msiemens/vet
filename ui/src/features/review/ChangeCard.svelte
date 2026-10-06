<script module lang="ts">
  export interface NonTextChange {
    path: string;
    kind: "mode" | "symlink" | "submodule" | "binary" | "unavailable";
    before: string;
    after: string;
    /** Binary images: show a before/after preview. */
    preview?: boolean;
    reason?: string;
  }
</script>

<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import type { HunkState } from "./editor/hunkBar";

  let { change, state, ontoggle }: { change: NonTextChange; state: HunkState; ontoggle: () => void } = $props();

  const TITLE = { mode: "File mode", symlink: "Symlink target", submodule: "Submodule commit", binary: "Binary file", unavailable: "Content unavailable" } as const;
  const LABEL = { unseen: "Mark seen", seen: "Mark unseen", oldBase: "Re-confirm seen" } as const;
</script>

<article class="card">
  <div class="vet-hunk vet-hunk-{state}">
    <span class="vet-hunk-text">{TITLE[change.kind]}</span>
    {#if state === "oldBase"}<span class="vet-hunk-old" title="Base changed since you marked this seen"><Icon name="reconfirm" size={13} stroke={2} /></span>{/if}
    <span class="path mono">{change.path}</span>
    <span class="spacer"></span>
    <button class="vet-hunk-btn vet-seen-toggle" type="button" aria-label={LABEL[state]} title={LABEL[state]} onclick={ontoggle}><Icon name="check" size={13} stroke={2} /></button>
    <button class="vet-hunk-btn" type="button" aria-label="Comment" title="Comment"><Icon name="comments" size={13} stroke={1.8} /></button>
  </div>
  <div class="body">
    {#if change.kind === "unavailable"}
      <span class="note"><Icon name="lock" size={14} />{change.reason}</span>
    {:else}
      <div class="line del"><span class="sign">−</span><span class="mono">{change.before}</span></div>
      <div class="line add"><span class="sign">+</span><span class="mono">{change.after}</span></div>
      {#if change.preview}
        <div class="previews">
          {#each ["Before", "After"] as side}
            <figure><div class="thumb"><Icon name="image" size={28} /></div><figcaption>{side}</figcaption></figure>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</article>

<style>
  .card { border: 1px solid var(--ln); border-radius: 6px; overflow: hidden; background: var(--ed); }
  .path { font-size: var(--fs-mono-s); color: var(--tx); }
  .spacer { flex: 1; }
  .body { display: flex; flex-direction: column; }
  .line { display: flex; gap: 8px; height: var(--h-line); line-height: var(--h-line); padding: 0 10px; font-size: var(--fs-mono); }
  .sign { width: 14px; text-align: center; }
  .add { background: var(--add); }
  .add .sign { color: var(--addt); }
  .del { background: var(--del); }
  .del .sign { color: var(--delt); }
  .note { display: flex; align-items: center; gap: 8px; padding: 8px 10px; color: var(--tx2); }
  .previews { display: flex; gap: 16px; padding: 10px; }
  figure { margin: 0; display: flex; flex-direction: column; gap: 4px; align-items: center; color: var(--tx3); font-size: var(--fs-mono-s); }
  .thumb { width: 120px; height: 80px; display: flex; align-items: center; justify-content: center; border: 1px dashed var(--ln); border-radius: var(--radius); background: var(--panel); }
</style>

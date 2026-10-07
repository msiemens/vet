<script lang="ts">
  import type { CommitUnit } from "../../generated/CommitUnit";
  import { shortTime } from "../../lib/shortTime";

  /** What the Changes tab says of the commit it shows: who wrote it, when, and what they said. */
  let { commit }: { commit: CommitUnit } = $props();

  const info = $derived(commit.commit);
  /** One person always gets the same colour. */
  const hue = $derived(Array.from(info.author, (c) => c.charCodeAt(0)).reduce((sum, c) => (sum * 31 + c) % 360, 7));
  const initial = $derived(Array.from(info.author)[0]?.toUpperCase());
  const parents = $derived(commit.kind.type === "merge" ? commit.parents.map((p) => p.slice(0, 7)) : []);
</script>

<header>
  <div class="top">
    <strong class="subject">{info.subject}</strong>
    <span class="who">
      <span class="avatar" style:--hue={hue} aria-hidden="true">{initial}</span>
      {info.author}
    </span>
    <span class="meta">{shortTime(info.time)}</span>
    <span class="mono meta">{info.shortId}</span>
  </div>
  {#if parents.length}
    <p class="parents selectable">Merge of <span class="mono">{parents.join(" and ")}</span></p>
  {/if}
  {#if commit.body.trim()}<p class="body selectable">{commit.body.trim()}</p>{/if}
</header>

<style>
  header {
    flex: none;
    padding: 8px 14px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--bg-editor);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .subject {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-ctl);
  }
  .who {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
  .avatar {
    display: inline-flex;
    justify-content: center;
    align-items: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: hsl(var(--hue) 45% 38%);
    color: white;
    font-size: 11px;
  }
  .meta {
    color: var(--text-2);
    font-size: var(--fs-mono-s);
    white-space: nowrap;
  }
  .parents,
  .body {
    margin: 4px 0 0;
    color: var(--text-2);
    font-size: var(--fs-ctl);
    white-space: pre-wrap;
  }
</style>

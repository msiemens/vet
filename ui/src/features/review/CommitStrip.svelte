<script lang="ts">
  import type { CommitUnit } from "../../generated/CommitUnit";
  import { shortTime } from "../../lib/shortTime";
  import Icon from "../../lib/ui/Icon.svelte";

  /**
   * What a file tab says of the commit it stays at: its short hash always, and once the commit is read
   * its subject, author and time. `id` is the full id.
   */
  let { id, commit }: { id: string; commit?: CommitUnit } = $props();

  const merge = $derived(commit?.kind.type === "merge");
</script>

<div class="strip" role="status" title="Comments are made in All changes">
  <span class="glyph"><Icon name={merge ? "merge" : "commit"} size={12} /></span>
  <span class="lead">{merge ? "This file as of merge" : "This file as of"}</span>
  <span class="mono hash">{commit?.commit.shortId ?? id.slice(0, 7)}</span>
  {#if commit}
    <span class="subject">{commit.commit.subject}</span>
    <span class="who">{commit.commit.author} · {shortTime(commit.commit.time, Date.now(), "detailed")}</span>
  {/if}
</div>

<style>
  .strip {
    container-type: inline-size;
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--h-ctl);
    box-sizing: border-box;
    padding: 0 12px;
    flex: none;
    min-width: 0;
    background: color-mix(in srgb, var(--accent) 11%, var(--bg-panel));
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-ctl);
  }
  .glyph {
    display: inline-flex;
  }
  .lead {
    color: var(--text-2);
    white-space: nowrap;
  }
  .hash {
    font-size: var(--fs-mono-s);
    font-weight: 500;
    white-space: nowrap;
  }
  .subject {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }
  .who {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
    white-space: nowrap;
  }
  /* Author and time go first, then the lead-in. */
  @container (max-width: 560px) {
    .who {
      display: none;
    }
  }
  @container (max-width: 440px) {
    .lead {
      display: none;
    }
  }
</style>

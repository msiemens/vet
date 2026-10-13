<script lang="ts">
  import type { CommitUnit } from "../../generated/CommitUnit";
  import { ciIcon } from "../../lib/ciStatus";
  import { shortTime } from "../../lib/shortTime";
  import Avatar from "../../lib/ui/Avatar.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { checks } from "../../state/checks.svelte";
  import { passed } from "../checks";

  /** What the Changes tab says of the commit it shows: who wrote it, when, and what they said. */
  let { commit }: { commit: CommitUnit } = $props();

  const info = $derived(commit.commit);
  const parents = $derived(commit.kind.type === "merge" ? commit.parents.map((p) => p.slice(0, 7)) : []);
  const checked = $derived(checks.of(info.id));
  const look = $derived(ciIcon(checked?.state));
  const count = $derived(checked ? passed(checked) : { passed: 0, total: 0 });
</script>

<header>
  <div class="top">
    <strong class="subject">{info.subject}</strong>
    <span class="who">
      <Avatar name={info.author} url={null} size={18} />
      {info.author}
    </span>
    <span class="meta">{shortTime(info.time, Date.now(), "detailed")}</span>
    <span class="mono meta">{info.shortId}</span>
    {#if checked && look}
      <span class="meta ci {look.tone}" title={`${count.passed} of ${count.total} checks passed`}>
        <Icon name={look.icon} size={12} stroke={2.2} />
        <span class="mono">{count.passed}/{count.total}</span>
      </span>
    {/if}
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
  .meta {
    color: var(--text-2);
    font-size: var(--fs-mono-s);
    white-space: nowrap;
  }
  .ci {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .ci.ok {
    color: var(--add-text);
  }
  .ci.failing {
    color: var(--del-text);
  }
  .ci.running {
    color: var(--old-base);
  }
  .ci.waiting {
    color: var(--text-3);
  }
  .parents,
  .body {
    margin: 4px 0 0;
    color: var(--text-2);
    font-size: var(--fs-ctl);
    white-space: pre-wrap;
  }
</style>

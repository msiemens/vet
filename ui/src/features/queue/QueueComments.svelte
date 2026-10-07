<script lang="ts">
  import type { Thread } from "../../generated/Thread";
  import Icon from "../../lib/ui/Icon.svelte";
  import { groupThreads, placeLabel, preview } from "../comments";

  let { threads, onopen }: { threads: readonly Thread[]; onopen: (thread: Thread) => void } = $props();

  const groups = $derived(groupThreads(threads));
  const sections = $derived(
    [
      { id: "detached", title: "Detached", list: groups.detached },
      { id: "moved", title: "Moved", list: groups.moved },
      { id: "changeset", title: "Changeset", list: groups.changeset },
      ...groups.files.map((f) => ({ id: `file:${f.path}`, title: f.path, list: f.threads })),
    ].filter((s) => s.list.length),
  );
  let selected = $state<number>();
</script>

{#each sections as section (section.id)}
  <section aria-label={section.title}>
    <div class="head">
      <span class="title" class:mono={section.id.startsWith("file:")}>{section.title}</span>
      <span class="mono n">{section.list.length}</span>
    </div>
    {#each section.list as thread (thread.id)}
      <button
        class="flat thread"
        class:on={thread.id === selected}
        class:resolved={thread.status === "resolved"}
        type="button"
        onclick={() => (selected = thread.id)}
        ondblclick={() => onopen(thread)}
        onkeydown={(e) => {
          if (e.key === "Enter") onopen(thread);
        }}
      >
        <span class="status"
          >{#if thread.status === "resolved"}<Icon name="check" size={12} stroke={2.2} />{:else}<Icon
              name="comments"
              size={12}
            />{/if}</span
        >
        <span class="preview">{preview(thread)}</span>
        <span class="mono place">{placeLabel(thread)}</span>
      </button>
    {/each}
  </section>
{:else}
  <p class="empty">No comments</p>
{/each}

<style>
  .head {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 6px 16px 2px;
    font-weight: 600;
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .n {
    font-weight: 400;
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .thread {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: var(--h-row);
    padding: 0 16px;
    box-sizing: border-box;
    border-radius: 0;
    text-align: left;
    font-size: var(--fs-ctl);
  }
  .thread.on {
    background: var(--toggle);
  }
  .status {
    display: inline-flex;
    color: var(--text-2);
  }
  .preview {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .resolved .preview,
  .resolved .status {
    color: var(--text-3);
  }
  .place {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .empty {
    margin: 12px 16px;
    color: var(--text-3);
  }
</style>

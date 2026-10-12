<script lang="ts">
  import { tick } from "svelte";
  import type { Thread } from "../../generated/Thread";
  import Icon from "../../lib/ui/Icon.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { groupThreads, preview, ThreadPlace } from "../comments";

  let {
    threads,
    onopen,
  }: {
    threads: readonly Thread[];
    /** Shows the thread in the review: a double click or Enter. */
    onopen: (thread: Thread) => void;
  } = $props();

  const groups = $derived(groupThreads(threads));
  const sections = $derived(
    [
      { id: "conflicts", title: "Conflicts", list: groups.conflicts },
      { id: "open", title: "Open", list: groups.open },
      { id: "resolved", title: "Resolved", list: groups.resolved },
    ].filter((s) => s.list.length),
  );
  /** The threads in the order the rows show them. */
  const flat = $derived(sections.flatMap((s) => s.list));
  /** Where the threads of each section start in `flat`. */
  const starts = $derived(sections.map((_, i) => sections.slice(0, i).reduce((n, s) => n + s.list.length, 0)));
  let selected = $state<number>();
  let highlighted = $state(-1);
  let list = $state<HTMLElement>();
  const nav = new ListNav<Thread>({
    element: () => list,
    items: () => flat,
    single: (thread) => (selected = thread.id),
    double: (thread) => onopen(thread),
    highlight: { get: () => highlighted, set: (i) => (highlighted = i) },
    reveal: (i) =>
      void tick().then(() =>
        list?.querySelector<HTMLElement>(`[data-index="${i}"]`)?.scrollIntoView({ block: "nearest" }),
      ),
  });
  $effect(() => () => nav.cancel());

  function onkeydown(e: KeyboardEvent) {
    if (!nav.keydown(e)) return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<ScrollArea bind:viewport={list} {onkeydown} {...nav.attributes}>
  {#each sections as section, at (section.id)}
    <section aria-label={section.title}>
      <div class="head">
        <span class="title">{section.title}</span>
        <span class="mono n">{section.list.length}</span>
      </div>
      {#each section.list as thread, i (thread.id)}
        <button
          class="flat thread"
          class:on={thread.id === selected}
          class:cur={starts[at] + i === highlighted}
          class:resolved={thread.status === "resolved"}
          type="button"
          tabindex="-1"
          data-index={starts[at] + i}
          onclick={(e) => nav.click(e, starts[at] + i)}
        >
          <span class="status"
            >{#if thread.status === "resolved"}<Icon name="check" size={12} stroke={2.2} />{:else}<Icon
                name="comments"
                size={12}
              />{/if}</span
          >
          <span class="preview">{preview(thread)}</span>
          <span class="mono place"><ThreadPlace {thread} /></span>
        </button>
      {/each}
    </section>
  {:else}
    <p class="empty">No comments</p>
  {/each}
</ScrollArea>

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
  /* The highlighted row, which Space and Enter act on, while the list has the focus. */
  :global([data-list]:focus-within) .thread.cur {
    background: var(--select);
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
    flex: 0 1 auto;
    min-width: 0;
    max-width: 50%;
    display: flex;
    font-size: var(--fs-mono-s);
  }
  .empty {
    margin: 12px 16px;
    color: var(--text-3);
  }
</style>

<script lang="ts">
  import type { Task } from "../../generated/Task";
  import Popover from "../../lib/ui/Popover.svelte";
  import ProgressBar from "../../lib/ui/ProgressBar.svelte";
  import { fractionOf, headline } from "../../state/taskHeadline";

  /** The running background tasks worth showing, oldest first. Nothing is drawn without any. */
  let { tasks }: { tasks: readonly Task[] } = $props();

  const head = $derived(headline(tasks));
  const newestFirst = $derived([...tasks].reverse());
  let open = $state(false);

  // The sheet goes with the last task, so it is not open when the next one starts.
  $effect(() => {
    if (!head) open = false;
  });
</script>

{#if head}
  <Popover bind:open above align="right">
    {#snippet trigger(toggle)}
      <button
        class="flat footer-item running"
        type="button"
        aria-expanded={open}
        title="Background tasks"
        onclick={toggle}
      >
        <span class="title">{head.title}</span>
        <span class="bar"><ProgressBar fraction={head.fraction} label={head.title} /></span>
        {#if head.more > 0}<span class="more">+{head.more} more</span>{/if}
      </button>
    {/snippet}
    <ul class="tasks">
      {#each newestFirst as task (task.id)}
        <li>
          <span class="name">{task.title}</span>
          {#if task.detail}<span class="detail" title={task.detail}>{task.detail}</span>{/if}
          <ProgressBar fraction={fractionOf(task)} label={task.title} />
        </li>
      {/each}
    </ul>
  </Popover>
{/if}

<style>
  .running {
    gap: 8px;
    min-width: 0;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar {
    width: var(--w-progress);
    flex: none;
  }
  .more {
    color: var(--text-3);
    flex: none;
  }
  .tasks {
    width: var(--w-tasks);
    max-width: 100%;
    margin: 0;
    padding: 6px 0;
    list-style: none;
    font-size: var(--fs-ctl);
  }
  li {
    display: grid;
    gap: 3px;
    padding: 6px 12px;
  }
  .name {
    color: var(--text);
  }
  .detail {
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

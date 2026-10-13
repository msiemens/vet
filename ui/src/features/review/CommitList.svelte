<script lang="ts">
  import type { CommitUnit } from "../../generated/CommitUnit";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import LineDelta from "../../lib/ui/LineDelta.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import SeenMeter from "../../lib/ui/SeenMeter.svelte";
  import { checks } from "../../state/checks.svelte";
  import { commitRow } from "./commitRows";

  let {
    commits,
    selected,
    onselect,
  }: {
    commits: CommitUnit[];
    selected?: number;
    /** The index into `commits`. */
    onselect: (index: number) => void;
  } = $props();

  const rows = $derived(commits.map((c) => commitRow(c)));

  /** Up and down move the selection through the list. */
  function move(e: KeyboardEvent, index: number) {
    const to = e.key === "ArrowDown" ? index + 1 : e.key === "ArrowUp" ? index - 1 : undefined;
    if (to === undefined || to < 0 || to >= rows.length) return;
    e.preventDefault();
    onselect(to);
    (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>(".row")[to]?.focus();
  }
</script>

<ScrollArea>
  <div class="list" role="listbox" aria-label="Commits">
    {#each rows as row, index (commits[index].commit.id)}
      <button
        class="flat pick row"
        class:on={index === selected}
        class:clean={row.clean}
        type="button"
        role="option"
        aria-selected={index === selected}
        title={row.notRemerged
          ? "Compared with its first parent: the automatic merge of its parents could not be made"
          : undefined}
        onclick={() => onselect(index)}
        onkeydown={(e) => move(e, index)}
      >
        <span class="mark">
          {#if row.merge}<Icon name="merge" size={12} />{:else}<span class="dot"></span>{/if}
        </span>
        <span class="text">
          <span class="first">
            <span class="subject">{row.subject}</span>
            <SeenMeter width={36} seen={row.seen} old={row.oldBase} />
          </span>
          <span class="second">
            <span class="mono hash">{row.shortId}</span>
            <span>{row.author}</span>
            <span>{row.when}</span>
            {#if row.clean}
              <span>clean merge</span>
            {:else}
              <LineDelta added={row.added} removed={row.removed} />
            {/if}
            <span class="grow"></span>
            <CiStatus state={checks.of(commits[index].commit.id)?.state} size={12} />
          </span>
        </span>
      </button>
    {/each}
  </div>
</ScrollArea>

<style>
  .list {
    display: flex;
    flex-direction: column;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    width: 100%;
    padding: 5px 10px 5px 8px;
    text-align: left;
    box-sizing: border-box;
    border-bottom: 1px solid var(--line);
    flex: none;
  }
  .mark {
    flex: none;
    display: inline-flex;
    justify-content: center;
    align-items: center;
    width: 14px;
    height: 16px;
    color: var(--text-3);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    border: 1.5px solid currentColor;
    box-sizing: border-box;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .first,
  .second {
    display: flex;
    align-items: center;
    gap: 8px;
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
  .second {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .hash {
    color: var(--text-2);
  }
  .clean .subject,
  .clean .mark {
    color: var(--text-3);
  }
</style>

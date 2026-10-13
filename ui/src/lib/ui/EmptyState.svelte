<!-- What a panel says where its list would be: nothing in it, why, or that it is on its way. In the bottom
     tools it is centred, in the side tools it starts at the left edge. A line of text alone is a plain note. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import Spinner from "./Spinner.svelte";

  let {
    title,
    text,
    icon,
    tone,
    busy = false,
    error = false,
    align = "center",
    actions,
    children,
  }: {
    title?: string;
    text?: string;
    icon?: IconName;
    /** Colours the icon: all is well, or something is wrong. */
    tone?: "ok" | "warn";
    /** Something is being worked out; the spinner stands where the icon would. */
    busy?: boolean;
    /** The text is a failure message. */
    error?: boolean;
    align?: "center" | "start";
    /** Buttons under the text. */
    actions?: Snippet;
    /** Text that is more than a string: after `text`. */
    children?: Snippet;
  } = $props();
</script>

<div class="empty {align}">
  {#if busy}<Spinner />{:else if icon}<span class="icon {tone}"><Icon name={icon} size={22} stroke={1.6} /></span>{/if}
  {#if title}<b>{title}</b>{/if}
  {#if text !== undefined || children}<span class="text selectable" class:err={error}>{text}{@render children?.()}</span
    >{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
    line-height: 1.5;
  }
  .center {
    align-items: center;
    justify-content: center;
    padding: 22px 24px;
    text-align: center;
  }
  .start {
    align-items: flex-start;
    padding: 14px 12px;
  }
  b {
    color: var(--text);
    font-size: var(--fs);
    font-weight: 600;
  }
  .text {
    max-width: 58ch;
  }
  .icon {
    display: inline-flex;
    color: var(--text-3);
  }
  .icon.ok {
    color: var(--ok);
  }
  .icon.warn {
    color: var(--warn);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .center .actions {
    justify-content: center;
  }
</style>

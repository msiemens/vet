<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import { platform } from "../../state/platform.svelte";
  import { threads } from "../../state/threads.svelte";
  import { outgoingComments, summarizeOutgoing } from "./outgoing";

  /** The toolbar's count of the viewer's comments that the platform does not have yet. */
  const list = $derived(outgoingComments(threads.items));
  const summary = $derived(summarizeOutgoing(list));
  const busy = $derived(platform.sending || summary.posting);
  const title = $derived(
    summary.failed > 0
      ? `${summary.failed} of ${summary.count} outgoing ${summary.count === 1 ? "comment" : "comments"} failed to post`
      : summary.count > 0
        ? `${summary.count} outgoing ${summary.count === 1 ? "comment" : "comments"}`
        : "No outgoing comments",
  );
</script>

<button
  class="ctl status"
  class:failed={summary.failed > 0}
  type="button"
  disabled={summary.count === 0}
  {title}
  aria-label={title}
>
  {#if busy}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name="outgoing" size={12} />{/if}
  <span class="mono">{summary.failed > 0 ? summary.failed : summary.count}</span>
</button>

<style>
  .status {
    margin-right: 6px;
  }
  .failed {
    color: var(--severity-error);
  }
  .spinner {
    width: 10px;
    height: 10px;
    box-sizing: border-box;
    border: 1.5px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>

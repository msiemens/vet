<script lang="ts">
  import type { QueueStatus } from "../../generated/QueueStatus";
  import { statusText } from "./queueRows";

  /** Where a pull request stands for the viewer; nothing is drawn without one. `title` is what it says on hover. */
  let { status, title }: { status?: QueueStatus; title?: string } = $props();
</script>

{#if status}<span class="chip {status}" {title}>{statusText(status)}</span>{/if}

<style>
  .chip {
    display: inline-block;
    max-width: 100%;
    box-sizing: border-box;
    padding: 2px 7px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--tone) 16%, transparent);
    color: var(--tone);
    font-size: var(--fs-mono-s);
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .draft {
    --tone: var(--status-draft);
  }
  .waitingForYou {
    --tone: var(--status-waiting);
  }
  .changesRequested {
    --tone: var(--status-changes);
  }
  .approved {
    --tone: var(--status-approved);
  }
  .blocked {
    --tone: var(--status-blocked);
  }
</style>

<script lang="ts">
  import type { StatusTone } from "./statusTone";

  /** A dot that colours a status; `hollow` is a ring, for a status that is still to come. `label` names it for assistive technology. */
  let {
    tone,
    hollow = false,
    label,
    title = label,
  }: { tone: StatusTone; hollow?: boolean; label?: string; title?: string } = $props();
</script>

<span
  class="dot"
  class:hollow
  data-tone={tone}
  role={label === undefined ? undefined : "img"}
  aria-label={label}
  {title}
></span>

<style>
  .dot {
    flex: none;
    display: inline-block;
    box-sizing: border-box;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    --tone: var(--text-3);
    background: var(--tone);
  }
  .hollow {
    background: transparent;
    border: 1.5px solid var(--tone);
  }
  [data-tone="ok"] {
    --tone: var(--ok);
  }
  [data-tone="warn"] {
    --tone: var(--warn);
  }
  [data-tone="error"] {
    --tone: var(--del-text);
  }
  [data-tone="note"] {
    --tone: var(--note);
  }
  [data-tone="none"] {
    --tone: transparent;
  }
</style>

<script lang="ts">
  import SeverityIcon from "../../lib/ui/SeverityIcon.svelte";
  import type { BaseNote } from "./scopeText";

  let { note }: { note: BaseNote } = $props();
</script>

<span class="base" class:alert={note.alert} title={note.tip} data-tauri-drag-region>
  {#if note.alert}<span class="warn" role="img" aria-label="HEAD no longer builds on this base"
      ><SeverityIcon severity="warning" /></span
    >{/if}{note.text}
</span>

<style>
  .base {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 6px;
    margin-right: 6px;
    border-radius: var(--radius);
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .base.alert {
    color: var(--text);
    background: color-mix(in srgb, var(--severity-warning) 16%, transparent);
  }
  .warn {
    display: inline-flex;
  }
</style>

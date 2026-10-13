<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import type { BaseNote } from "./scopeText";

  let { note }: { note: BaseNote } = $props();
</script>

<span class="base" class:alert={note.alert} title={note.tip} data-tauri-drag-region>
  {#if note.alert}<span class="warn" aria-label="HEAD no longer builds on this base"
      ><Icon name="warning" size={13} /></span
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
    background: color-mix(in srgb, var(--old-base) 16%, transparent);
  }
  .warn {
    display: inline-flex;
    color: var(--old-base);
  }
</style>

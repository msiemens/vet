<script lang="ts">
  import { deltaParts } from "./lineDelta";

  /** The lines a change added and removed, as `+12 −3`. */
  let { added, removed }: { added: number; removed: number } = $props();
  const [plus, minus] = $derived(deltaParts(added, removed));
</script>

<span class="mono delta"><span class="add">{plus}</span> <span class="del">{minus}</span></span>

<style>
  .delta {
    font-size: var(--fs-mono-s);
    white-space: nowrap;
  }
  .add {
    color: var(--add-text);
  }
  .del {
    color: var(--del-text);
  }
</style>

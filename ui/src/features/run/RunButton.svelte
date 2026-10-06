<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import SplitButton from "../../lib/ui/SplitButton.svelte";
  import { runs } from "../../state/runs.svelte";
  import { sourceOf } from "./command";

  const primary = $derived(runs.primary?.target);
</script>

<SplitButton
  label="Run"
  items={runs.targets.map(({ target }) => ({ id: target.id, label: target.label, detail: sourceOf(target.id) }))}
  onmain={() => (primary ? void runs.start(primary.id) : (runs.editing = true))}
  onselect={(id) => void runs.start(id)}
>
  {#snippet main()}
    <span class="play"><Icon name="play" size={12} /></span>{primary?.label ?? "Run"}
  {/snippet}
  {#snippet menuFooter(close)}
    <button
      type="button"
      class="flat edit"
      onclick={() => {
        close();
        runs.editing = true;
      }}>Edit targets…</button
    >
  {/snippet}
</SplitButton>

<style>
  .play {
    display: inline-flex;
    color: var(--ok);
  }
  .edit {
    width: 100%;
    height: var(--h-row);
    text-align: left;
    padding: 0 12px;
    font-size: var(--fs-ctl);
  }
</style>

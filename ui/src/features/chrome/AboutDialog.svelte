<script lang="ts">
  import Dialog from "../../lib/ui/Dialog.svelte";

  let { onclose }: { onclose: () => void } = $props();
</script>

<Dialog title="About Vet" {onclose} width="360px">
  <p class="name">Vet</p>
  <p class="text">Review a working tree against a base: hunks, coverage and what is still unseen.</p>
  {#snippet actions()}
    <button class="ctl" type="button" onclick={onclose}>Close</button>
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
  }
  .name {
    font-size: var(--fs-about);
    font-weight: 600;
    margin-bottom: 6px;
  }
  .text {
    color: var(--text-2);
  }
</style>

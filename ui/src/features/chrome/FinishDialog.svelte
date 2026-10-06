<script lang="ts">
  import Dialog from "../../lib/ui/Dialog.svelte";

  let { onfinish, onclose }: { onfinish: () => void; onclose: () => void } = $props();
</script>

<Dialog title="Finish review" {onclose} width="380px">
  <p>
    The review ends here, with changes left unseen if there are any. Marks and comments leave the screen, and the next
    mark starts a new review.
  </p>
  {#snippet actions()}
    <button class="ctl" type="button" onclick={onclose}>Cancel</button>
    <button class="ctl" type="button" onclick={onfinish}>Finish review</button>
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
    color: var(--text-2);
  }
</style>

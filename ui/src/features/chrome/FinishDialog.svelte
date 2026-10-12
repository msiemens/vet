<script lang="ts">
  import Dialog from "../../lib/ui/Dialog.svelte";
  import { flaggedWarning } from "./finishWarning";

  let {
    flagged,
    ongoto,
    onfinish,
    onclose,
  }: {
    /** How many hunks are flagged for later review. */
    flagged: number;
    /** Goes to the first flagged hunk. */
    ongoto: () => void;
    onfinish: () => void;
    onclose: () => void;
  } = $props();
  const warning = $derived(flaggedWarning(flagged));
</script>

<Dialog title="Finish review" {onclose} width="380px">
  <p>
    The review ends here, with changes left unseen if there are any. Marks and comments leave the screen, and the next
    mark starts a new review.
  </p>
  {#if warning}
    <p class="warn">{warning}</p>
  {/if}
  {#snippet actions()}
    <button class="ctl" type="button" onclick={onclose}>Cancel</button>
    {#if warning}
      <button class="ctl" type="button" onclick={ongoto}>Go to Flagged Hunk</button>
    {/if}
    <button class="ctl" type="button" onclick={onfinish}>Finish review</button>
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
    color: var(--text-2);
  }
  p + p {
    margin-top: 8px;
  }
  .warn {
    color: var(--warn);
  }
</style>

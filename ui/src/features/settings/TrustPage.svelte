<script lang="ts">
  import Checkbox from "../../lib/ui/Checkbox.svelte";

  let {
    trusted,
    ontrusted,
  }: {
    /** Whether the repository is marked trusted. */
    trusted: boolean;
    /** Marks the repository trusted or not. */
    ontrusted: (trusted: boolean) => void;
  } = $props();
</script>

<div class="page">
  <Checkbox checked={trusted} onchange={ontrusted}>Trust this project</Checkbox>
  <p class="line">Lets Vet run its install scripts and build tools.</p>
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-ctl);
  }
  /* Under the label, in line with its text. */
  .line {
    margin: 0 0 0 28px;
    color: var(--text-3);
    font-size: var(--fs-mono);
  }
</style>

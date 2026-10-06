<script lang="ts">
  import { runs } from "../../state/runs.svelte";
  import ApprovalDialog from "./ApprovalDialog.svelte";
  import TargetsDialog from "./TargetsDialog.svelte";
</script>

{#if runs.approval}
  {#key runs.approval.hash}<ApprovalDialog approval={runs.approval} />{/key}
{:else if runs.editing}
  <TargetsDialog />
{/if}

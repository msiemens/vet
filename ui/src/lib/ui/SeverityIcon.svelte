<script lang="ts">
  import type { Severity } from "../../generated/Severity";
  import Icon from "./Icon.svelte";
  import { severityColor, severityIcon } from "./severity";

  let { severity, size = 14 }: { severity: Severity; size?: number } = $props();
</script>

<span class="severity-icon" style:color={severityColor(severity)}><Icon name={severityIcon(severity)} {size} /></span>

<style>
  .severity-icon {
    display: inline-flex;
    flex: none;
  }
</style>

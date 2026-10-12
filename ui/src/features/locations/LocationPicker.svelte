<script lang="ts">
  // Shows the popup while the location picker has something to say.
  import { locationPicker } from "../../state/locationPicker.svelte";
  import PickerPopup from "./PickerPopup.svelte";

  let { statusOf, onsettings }: { statusOf: (path: string) => string | undefined; onsettings: () => void } = $props();
</script>

<!-- Each search is a popup of its own: what was typed in one is not carried over. -->
{#if locationPicker.view}
  {#key locationPicker.view}
    <PickerPopup view={locationPicker.view} {statusOf} {onsettings} />
  {/key}
{/if}

<script lang="ts">
  import { onMount } from "svelte";
  import { SOFT_WRAPS, type SoftWrap } from "../../lib/softWrap";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { backendSettings } from "../../state/backendSettings.svelte";

  let { softWrap, onsoftwrap }: { softWrap: SoftWrap; onsoftwrap: (wrap: SoftWrap) => void } = $props();

  let problem = $state<string>();

  onMount(() => void backendSettings.load());

  /** A whole number of days, at least one; anything else puts the kept number back in the field. */
  async function setDays(input: HTMLInputElement) {
    const days = Number(input.value);
    const kept = backendSettings.settings?.finishedReviewDays;
    if (!Number.isInteger(days) || days < 1) {
      input.value = String(kept ?? "");
      problem = "Finished reviews are kept for a whole number of days, at least 1.";
      return;
    }
    if (days === kept) return;
    problem = await backendSettings.change({ finishedReviewDays: days });
  }
</script>

<div class="page">
  <div class="row">
    <span>Soft wrap</span>
    <Segmented label="Soft wrap" value={softWrap} onchange={onsoftwrap} options={SOFT_WRAPS} />
  </div>
  <div class="row">
    <label for="finished-review-days">Keep finished reviews</label>
    <div class="value">
      <input
        id="finished-review-days"
        class="field"
        type="number"
        min="1"
        step="1"
        disabled={!backendSettings.settings}
        value={backendSettings.settings?.finishedReviewDays ?? ""}
        onchange={(e) => void setDays(e.currentTarget)}
      />
      <span>days</span>
    </div>
  </div>
  {#if backendSettings.loadError}<p class="settings-problem">{backendSettings.loadError}</p>{/if}
  {#if problem}<p class="settings-problem" role="alert">{problem}</p>{/if}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 16px;
    font-size: var(--fs-ctl);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .value {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
  }
  .value input {
    width: 64px;
  }
  p {
    margin: 0;
  }
</style>

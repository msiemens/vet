<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import { platform } from "../../state/platform.svelte";
  import { connectionLabel } from "./platformLabels";
  import NotificationList from "./NotificationList.svelte";

  const status = $derived(platform.status);
  const label = $derived(status && connectionLabel(status));
  let listing = $state(false);

  const signIn = () => platform.openAccounts();
</script>

{#if label && status}
  <button class="flat footer-item platform" class:warn={label.warn} type="button" title={label.title} onclick={signIn}>
    <Icon name={status.kind === "github" ? "github" : "azureDevOps"} size={12} />
    <span class="dot" class:ok={!label.warn}></span>
    {label.text}
  </button>
{/if}
{#if platform.notified.length}
  <span class="bell">
    <button
      class="flat footer-item"
      type="button"
      aria-label="Notifications"
      aria-expanded={listing}
      title="Notifications"
      onclick={() => (listing = !listing)}
    >
      <Icon name="bell" size={13} />
    </button>
    {#if listing}<NotificationList onclose={() => (listing = false)} onsignin={signIn} />{/if}
  </span>
{/if}

<style>
  .platform.warn {
    color: var(--warn);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--warn);
  }
  .dot.ok {
    background: var(--ok);
  }
  .bell {
    position: relative;
    display: flex;
  }
</style>

<script lang="ts">
  import type { Snippet } from "svelte";
  import IconButton from "./IconButton.svelte";
  import SeverityIcon from "./SeverityIcon.svelte";

  let {
    message,
    ondismiss,
    details,
    actions = [],
    extra,
    tone = "caution",
    icon = false,
    panel = false,
  }: {
    message: string;
    /** What it tints with: amber for a caution, blue for news, orange for a warning (the problems'
     * warning colour), red for something that failed. */
    tone?: "caution" | "info" | "warn" | "bad";
    /** The tone's icon before the message. */
    icon?: boolean;
    /** A notice inside a panel, a size smaller, rather than one under the header. */
    panel?: boolean;
    /** Without it the banner stays while what it says holds. */
    ondismiss?: () => void;
    details?: Snippet;
    /** Buttons next to the message. */
    actions?: { label: string; onclick: () => void }[];
    /** Under the message. */
    extra?: Snippet;
  } = $props();

  const TONES = {
    info: { color: "var(--severity-info)", severity: "info" },
    warn: { color: "var(--severity-warning)", severity: "warning" },
    caution: { color: "var(--old-base)", severity: "warning" },
    bad: { color: "var(--del-text)", severity: "error" },
  } as const;

  let expanded = $state(false);
</script>

<div class="banner" class:panel role="status" style:--tone={TONES[tone].color}>
  <div class="line">
    {#if icon}<span class="icon"><SeverityIcon severity={TONES[tone].severity} /></span>{/if}
    <div class="msg">
      <span class="selectable">{message}</span>
      {@render extra?.()}
    </div>
    {#each actions as action (action.label)}
      <button class="flat" type="button" onclick={action.onclick}>{action.label}</button>
    {/each}
    {#if details}
      <button class="flat" type="button" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
        {expanded ? "Hide details" : "Details"}
      </button>
    {/if}
    {#if ondismiss}
      <div class="dismiss">
        <IconButton icon="close" label="Dismiss" size={20} iconSize={12} onclick={ondismiss} />
      </div>
    {/if}
  </div>
  {#if details && expanded}
    <div class="details selectable">{@render details()}</div>
  {/if}
</div>

<style>
  .banner {
    flex: none;
    padding: 4px 6px 4px 12px;
    background: color-mix(in srgb, var(--tone) 14%, var(--bg-panel));
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-ctl);
  }
  .banner.panel {
    padding: 6px 12px;
    border-bottom-color: var(--line-soft);
    font-size: 12px;
    color: var(--text-2);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .icon {
    align-self: flex-start;
    display: inline-flex;
    margin: 2px 4px 0 0;
  }
  .msg {
    margin-right: auto;
    padding-right: 10px;
  }
  /* The actions are plain text until pointed at; the hover and pressed boxes are the banner's own
     tone, a little stronger than its background. */
  button.flat {
    height: 20px;
    box-sizing: border-box;
    padding: 0 8px;
    border-radius: var(--radius);
    color: var(--text-2);
    line-height: 1;
  }
  button.flat:enabled:hover {
    background: color-mix(in srgb, var(--tone) 26%, var(--bg-panel));
  }
  button.flat:enabled:active {
    background: color-mix(in srgb, var(--tone) 38%, var(--bg-panel));
  }
  .dismiss {
    display: flex;
    margin-left: 4px;
  }
  .details {
    max-height: 30vh;
    overflow: auto;
    padding: 4px 0 4px 0;
    color: var(--text-2);
  }
</style>

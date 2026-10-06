<script lang="ts">
  import { ICONS, SOLID, type IconName } from "./icons";

  let { name, size = 14, stroke = 1.6 }: { name: IconName; size?: number; stroke?: number } = $props();
  const solid = $derived(SOLID.has(name));
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={solid ? "currentColor" : "none"}
  stroke={solid ? "none" : "currentColor"}
  stroke-width={stroke}
  aria-hidden="true"
>
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- static markup from icons.ts, never user input -->
  {@html ICONS[name]}
</svg>

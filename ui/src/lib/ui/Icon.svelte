<script lang="ts">
  import { ICONS, iconFrame, type IconName } from "./icons";

  let { name, size = 14, stroke = 1.6 }: { name: IconName; size?: number; stroke?: number } = $props();
  const frame = $derived(iconFrame(name));
</script>

<svg
  width={size}
  height={size}
  viewBox={frame.viewBox}
  fill={frame.fill}
  stroke={frame.stroke}
  stroke-width={stroke}
  aria-hidden="true"
>
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- static markup from icons.ts, never user input -->
  {@html ICONS[name]}
</svg>

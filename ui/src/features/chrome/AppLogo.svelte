<script lang="ts">
  let { size = 16 }: { size?: number } = $props();
</script>

<!-- The app's V: the removed (red) and added (green) arms of the icon, without its background.
     Part of the title bar's drag region. -->
<svg class="logo" viewBox="24 50 208 168" width={size} height={size} aria-hidden="true" data-tauri-drag-region>
  <path
    class="del"
    d="M25.67 54.53A1 1 0 0 1 26.55 53.05L70.70 53.05A15 15 0 0 1 84.26 61.63L124.25 146.16L147.73 202.64L140.92 214.85A0.6 0.6 0 0 1 140.39 215.16L114.09 215.16A0.6 0.6 0 0 1 113.56 214.85Z"
  />
  <path
    class="add"
    d="M172.93 60.43A15 15 0 0 1 186.19 52.45L230.83 52.45A0.4 0.4 0 0 1 231.18 53.04L148.90 200.54L126.82 147.57Z"
  />
</svg>

<style>
  .logo {
    flex: none;
  }
  /* Presses land on the svg, which carries the drag attribute. */
  path {
    pointer-events: none;
  }
  .del {
    fill: var(--del-text);
  }
  .add {
    fill: var(--add-text);
  }
</style>

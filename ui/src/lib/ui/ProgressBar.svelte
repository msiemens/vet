<script lang="ts">
  /** A thin bar: `fraction` in 0..1 fills it, none sweeps to show activity. Width follows the container. */
  let { fraction, label }: { fraction?: number; label?: string } = $props();
</script>

<span
  class="bar"
  role="progressbar"
  aria-label={label}
  aria-valuemin={fraction === undefined ? undefined : 0}
  aria-valuemax={fraction === undefined ? undefined : 100}
  aria-valuenow={fraction === undefined ? undefined : Math.round(fraction * 100)}
>
  {#if fraction === undefined}
    <span class="sweep"></span>
  {:else}
    <span class="fill" style:width="{fraction * 100}%"></span>
  {/if}
</span>

<style>
  .bar {
    position: relative;
    display: block;
    height: var(--h-progress);
    border-radius: var(--h-progress);
    background: var(--track);
    overflow: hidden;
  }
  .fill,
  .sweep {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    background: var(--accent);
  }
  .fill {
    transition: width 0.25s linear;
  }
  .sweep {
    width: 40%;
    animation: sweep 1.4s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .sweep {
      animation: none;
      width: 100%;
      opacity: 0.5;
    }
    .fill {
      transition: none;
    }
  }
</style>

<script lang="ts">
  /** A small dotted ring that turns: something is being worked out where a count will be. */
  let { label }: { label?: string } = $props();
</script>

<span class="spin" role="img" aria-label={label} title={label}></span>

<style>
  .spin {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px dotted var(--text-3);
    box-sizing: content-box;
    animation: turn 2.4s linear infinite;
  }
  @keyframes turn {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation: none;
    }
  }
</style>

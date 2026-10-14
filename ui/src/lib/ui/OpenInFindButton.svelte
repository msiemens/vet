<script lang="ts">
  // Keeps the results of a search in the Find window; `keys` do it from the keyboard.
  import Kbd from "./Kbd.svelte";

  let {
    keys,
    title,
    disabled = false,
    onclick,
  }: { keys: string; title: string; disabled?: boolean; onclick: () => void } = $props();
</script>

<button class="ctl keep" type="button" {title} {disabled} {onclick}>
  Open in Find window <Kbd {keys} />
</button>

<style>
  .keep {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: var(--h-ctl-s);
    padding: 0 9px;
    font-size: var(--fs-mono-s);
  }
</style>

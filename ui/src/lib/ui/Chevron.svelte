<script lang="ts">
  // The column of a row that opens and shuts what it holds. Without `open` the column stays, empty,
  // so the names of the rows above and below line up.
  import Icon from "./Icon.svelte";

  let { open }: { open?: boolean } = $props();
</script>

<span class="chev"
  >{#if open !== undefined}<Icon name={open ? "open" : "closed"} size={10} />{/if}</span
>

<style>
  .chev {
    flex: none;
    width: var(--w-chev);
    display: inline-flex;
    justify-content: center;
  }
</style>

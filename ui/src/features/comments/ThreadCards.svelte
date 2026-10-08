<script lang="ts">
  import type { Thread } from "../../generated/Thread";

  /**
   * Threads drawn as the cards the diff shows them in, with their replies and Resolve. `card` draws one;
   * `toggle` opens or folds a resolved thread.
   */
  let {
    list,
    card,
  }: {
    list: Thread[];
    card: (thread: Thread, open: boolean, toggle: () => void) => HTMLElement;
  } = $props();

  /** Resolved threads the viewer opened; the others are folded to a line. */
  let opened = $state<number[]>([]);

  const toggle = (id: number) => (opened = opened.includes(id) ? opened.filter((t) => t !== id) : [...opened, id]);
</script>

{#each list as thread (thread.id)}
  <div
    class="card"
    {@attach (box: HTMLElement) => {
      box.replaceChildren(card(thread, opened.includes(thread.id), () => toggle(thread.id)));
    }}
  ></div>
{/each}

<style>
  /* The card keeps to the panel's width and scrolls with it, unlike in an editor. */
  .card :global(.vet-thread) {
    position: static;
    width: auto;
  }
  .card + .card :global(.vet-thread) {
    border-top: none;
  }
</style>

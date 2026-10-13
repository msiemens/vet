<script lang="ts">
  import type { Thread } from "../../generated/Thread";

  /**
   * Threads drawn as the cards the diff shows them in. `card` draws one; `toggle` opens or folds a resolved
   * thread. The owner may keep which resolved threads are open (`isOpen`, `ontoggle`), else the list does.
   */
  let {
    list,
    card,
    isOpen,
    ontoggle,
  }: {
    list: Thread[];
    card: (thread: Thread, open: boolean, toggle: () => void) => HTMLElement;
    isOpen?: (thread: Thread) => boolean;
    ontoggle?: (thread: Thread) => void;
  } = $props();

  /** Resolved threads the viewer opened; the others are folded to a line. */
  let opened = $state<number[]>([]);

  const toggle = (id: number) => (opened = opened.includes(id) ? opened.filter((t) => t !== id) : [...opened, id]);
  const open = (thread: Thread) => (isOpen ? isOpen(thread) : opened.includes(thread.id));
  const flip = (thread: Thread) => (ontoggle ? ontoggle(thread) : toggle(thread.id));
</script>

{#each list as thread (thread.id)}
  <div
    class="card"
    {@attach (box: HTMLElement) => {
      box.replaceChildren(card(thread, open(thread), () => flip(thread)));
    }}
  ></div>
{/each}

<style>
  /* The card keeps to the panel's width and scrolls with it, unlike in an editor, and fills it. */
  .card :global(.vet-thread) {
    position: static;
    width: auto;
  }
  .card :global(.vet-thread-card) {
    max-width: none;
  }
</style>

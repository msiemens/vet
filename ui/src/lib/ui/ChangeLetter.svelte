<script lang="ts">
  import type { StatusLetter } from "../changeLetter";

  /**
   * The column that ends a row with how its file or symbol changed. Without a letter it keeps the
   * column's width, so the rows above and below line up.
   */
  let { letter, title }: { letter?: StatusLetter; title?: string } = $props();
</script>

<span class="mono change-letter" data-letter={letter} {title}>{letter ?? ""}</span>

<style>
  .change-letter {
    flex: none;
    display: inline-flex;
    justify-content: center;
    width: 10px;
    font-size: var(--fs-xs);
  }
  [data-letter="M"] {
    color: var(--old-base);
  }
  [data-letter="A"] {
    color: var(--add-text);
  }
  [data-letter="D"] {
    color: var(--del-text);
  }
  [data-letter="R"] {
    color: var(--text-2);
  }
</style>

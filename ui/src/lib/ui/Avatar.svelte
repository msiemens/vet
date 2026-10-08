<script lang="ts">
  import { avatarColor } from "../avatar";

  let { name, url, size = 16 }: { name: string; url: string | null; size?: number } = $props();
</script>

<!-- The platform's picture of a person, or a disc with their initials. -->
{#if url}
  <img src={url} alt="" style:width="{size}px" style:height="{size}px" />
{:else}
  <span
    class="initials"
    style:width="{size}px"
    style:height="{size}px"
    style:font-size="{Math.round(size / 2)}px"
    style:background="var(--avatar-{avatarColor(name)})"
  >
    {name.slice(0, 2).toUpperCase()}
  </span>
{/if}

<style>
  img,
  .initials {
    flex: none;
    border-radius: 50%;
  }
  img {
    object-fit: cover;
  }
  .initials {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--on-avatar);
    font-weight: 600;
  }
</style>

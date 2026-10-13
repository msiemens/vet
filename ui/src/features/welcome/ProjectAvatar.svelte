<script lang="ts">
  import { avatarColor } from "../../lib/avatar";
  import { repoName } from "../../lib/paths";
  import { initials } from "./recentList";

  /** A project's tile: its initials on a colour its path picks. */
  let { path }: { path: string } = $props();
</script>

<span class="avatar" style:background="var(--avatar-{avatarColor(path)})">{initials(repoName(path))}</span>

<style>
  .avatar {
    flex: none;
    width: var(--s-avatar);
    height: var(--s-avatar);
    display: grid;
    place-items: center;
    border-radius: var(--radius-sheet);
    color: var(--on-avatar);
    font-size: 12px;
    font-weight: 600;
  }
</style>

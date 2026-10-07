<script lang="ts">
  import { fileIcon, folderIcon } from "../fileIcons";
  import { FILE_ICON_URLS } from "./fileIconUrls";

  /** `name` is a file's name, or a folder's (also a chain like `crates/vet-core`) when `folder` is set. */
  let {
    name,
    folder = false,
    open = false,
    size = 14,
  }: { name: string; folder?: boolean; open?: boolean; size?: number } = $props();
  const url = $derived(FILE_ICON_URLS[folder ? folderIcon(name, open) : fileIcon(name)]);
</script>

<img src={url} width={size} height={size} alt="" draggable="false" />

<style>
  img {
    flex: none;
    display: block;
  }
</style>

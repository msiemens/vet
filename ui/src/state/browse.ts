import { openUrl } from "./host";
import { notices } from "./notices.svelte";

/** Shows a web address in the browser; says so if the system would not. */
export function browse(url: string): void {
  void openUrl(url).catch((e: unknown) => notices.failed("Could not open the browser", e));
}

import { notices } from "../../state/notices.svelte";

/** Puts `text` on the clipboard; says so if it could not. */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    notices.show("Could not copy to the clipboard");
  }
}

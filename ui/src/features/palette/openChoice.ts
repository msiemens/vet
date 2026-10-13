// What choosing a result of Search Everywhere does, once the popup has closed and the focus is back.
// The owner of the review screen gives the means; what to do with each kind of result is decided here.

import type { SymbolChange } from "../../generated/SymbolChange";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { Choice } from "./choice.ts";

export interface ChoiceHost {
  showReview(): void;
  /** A commit is on screen rather than the changeset as a whole. */
  commitShown(): boolean;
  /** Puts the changeset as a whole on screen; resolves once it is, and the review has drawn it. */
  showWhole(): Promise<void>;
  /** Opens the file's tab as the view on screen has it: a commit's, if one is on screen. `line` is 0-based. */
  openOnScreen(path: string, line?: number): void;
  /** Opens the file's tab of the changeset itself, never one pinned to a commit. */
  openUnpinned(path: string, line: number): void;
  /** Scrolls the Changes tab to a changed file. */
  revealFile(path: string): void;
  /** Selects the changed symbol and shows it where it is reviewed, in the Changes tab. */
  revealSymbol(symbol: SymbolInfo, change: SymbolChange): void;
  openChangeset(id: string): void;
  openThread(id: number): void;
  run(command: string): void;
}

/** Does what `choice` asks. Whatever is shown in the Changes tab is shown on the changeset as a whole. */
export async function openChoice(choice: Choice, host: ChoiceHost): Promise<void> {
  switch (choice.type) {
    case "file":
      host.showReview();
      if (!choice.inChanges) {
        host.openOnScreen(choice.path, choice.line);
        return;
      }
      await showWholeIfCommit(host);
      host.revealFile(choice.path);
      return;
    case "symbol": {
      host.showReview();
      const { symbol, change } = choice;
      // A removed symbol has no text to open, so it is shown where it is reviewed.
      if (change !== undefined && (change === "removed" || choice.reveal)) {
        await showWholeIfCommit(host);
        host.revealSymbol(symbol, change);
        return;
      }
      // The symbol was found in the changeset's target, so its file opens there, never at a pinned commit.
      host.openUnpinned(symbol.id.path, symbol.span.start.line);
      return;
    }
    case "changeset":
      host.showReview();
      host.openChangeset(choice.id);
      return;
    case "thread":
      host.openThread(choice.id);
      return;
    case "action":
      host.run(choice.id);
      return;
  }
}

/** A commit on screen lists only its own changes, in which a symbol or file of the changeset may be missing. */
async function showWholeIfCommit(host: ChoiceHost) {
  if (host.commitShown()) await host.showWhole();
}

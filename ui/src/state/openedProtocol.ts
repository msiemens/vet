// How the changeset on view is switched. A changeset this window asked the backend to open is not the one
// on view from the request until the backend has announced it (`changesetOpened`, which precedes the
// answer) or answered without: the request failed or the changeset was on view already. The session of the
// changeset left is ended at the request, so nothing of it is shown while the next one loads.

import type { ChangesetKey } from "../generated/ChangesetKey";
import { keyId } from "./changesetKey.ts";

/** What happens to the changeset on view: another is asked for, announced to be fetched, or the ask came to nothing. */
export type Opening = { type: "requested" } | { type: "opened"; changeset: ChangesetKey } | { type: "unannounced" };

/** What the protocol does to the window; the holder of the session implements it. */
export interface OpenedEffects {
  /** Ends the session on view, if there is one. */
  dispose(): void;
  /** Makes the session of the changeset the backend announced, at `revision`. It does not run yet. */
  build(key: ChangesetKey, revision: number): void;
  /** Runs the session `build` made. */
  start(revision: number): void;
  /** Tells whoever follows the switch. */
  notify(opening: Opening): void;
  /** Whether an open request is out, which the views show as loading. */
  busy(on: boolean): void;
  /** Reads the manifest and builds the session from what the backend has. */
  read(): Promise<void>;
}

export class OpenedProtocol {
  private seq = 0;
  private active = false;
  /** The changeset the latest request asked for. */
  private wanted = "";
  /** Counts what makes a read underway outdated: a request, an announcement, a close and a newer read. */
  private epoch = 0;
  private effects: OpenedEffects;

  constructor(effects: OpenedEffects) {
    this.effects = effects;
  }

  /** A request for `key` is going out; the token goes to `answered` when it is answered. */
  request(key: ChangesetKey): number {
    this.active = true;
    this.wanted = keyId(key);
    this.epoch++;
    this.effects.busy(true);
    this.effects.dispose();
    this.effects.notify({ type: "requested" });
    return ++this.seq;
  }

  /**
   * The backend announced `key` at `revision`. Its session is built, not while a newer request is
   * pending, whose own announcement follows (an earlier request's would show its changeset for a moment).
   * A changeset nobody here asked for (another window did) is built as well.
   */
  announced(key: ChangesetKey, revision: number): void {
    if (this.active) {
      if (keyId(key) !== this.wanted) return;
      this.active = false;
      this.effects.busy(false);
    }
    this.epoch++;
    this.effects.dispose();
    this.effects.build(key, revision);
    this.effects.notify({ type: "opened", changeset: key });
    this.effects.start(revision);
  }

  /**
   * The request `token` was answered. When nothing was announced for it (it failed, or the changeset was on
   * view already), the manifest is read and the session built from it. A newer request still pending stays
   * pending.
   */
  async answered(token: number): Promise<void> {
    if (token !== this.seq || !this.active) return;
    this.active = false;
    this.effects.busy(false);
    this.effects.notify({ type: "unannounced" });
    await this.effects.read();
  }

  /** A read of the manifest begins; hand the ticket to `readCurrent` once it is answered. */
  beginRead(): number {
    return ++this.epoch;
  }

  /** Whether nothing has been asked for, announced or closed since the read `ticket` began, so it may build a session. */
  readCurrent(ticket: number): boolean {
    return ticket === this.epoch;
  }

  /** The project closed: the session ends, and an open request or read still out counts for nothing. */
  close(): void {
    this.epoch++;
    if (this.active) this.effects.busy(false);
    this.active = false;
    this.effects.dispose();
  }
}

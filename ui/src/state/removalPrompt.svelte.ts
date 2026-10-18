// Removing a project, which the viewer confirms first: the open question that `RemoveProjectDialog`
// shows and answers, and the removal that follows a yes.

import { repoName } from "../lib/paths";
import { notices } from "./notices.svelte";
import { lostItems } from "./removal";
import { session } from "./session.svelte";

export interface RemoveQuestion {
  root: string;
  name: string;
  /** What removing the project loses, one line each. */
  lost: string[];
}

class RemovePrompt {
  /** The open question; none while there is none. */
  asking = $state<RemoveQuestion>();
  private waiting?: (remove: boolean) => void;

  /** Asks, resolving with whether the project may be removed. A question asked while another is open is answered no. */
  ask = (question: RemoveQuestion): Promise<boolean> => {
    if (this.waiting) return Promise.resolve(false);
    this.asking = question;
    return new Promise((resolve) => (this.waiting = resolve));
  };

  /** The viewer chose: Remove, or Cancel. */
  answer(remove: boolean) {
    const waiting = this.waiting;
    this.asking = undefined;
    this.waiting = undefined;
    waiting?.(remove);
  }
}

export const removePrompt = new RemovePrompt();

/** Removes the project at `root` after the viewer confirmed what that loses. The events refresh everything that shows it. */
export async function removeProject(root: string) {
  const name = repoName(root);
  const backend = session.connected();
  try {
    const facts = await backend.projectStorage(root);
    if (!(await removePrompt.ask({ root, name, lost: lostItems(facts) }))) return;
    await backend.removeProject(root);
  } catch (e) {
    notices.failed(`Cannot remove ${name}`, e);
  }
}

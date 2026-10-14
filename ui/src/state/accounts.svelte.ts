// The credentials the backend uses against hosting platforms, per host and not per repository. The
// backend keeps tokens in memory only; the host's keychain keeps the personal access tokens, and this
// store hands them over when the connection is up and when the backend asks (`credentialsNeeded`).

import type { Account } from "../generated/Account";
import { errorText } from "../lib/errorText";
import type { Backend } from "./backend";
import { removeStoredCredential, storeCredential, storedCredential, storedCredentials } from "./host";
import { notices } from "./notices.svelte";
import { Resource } from "./resource";
import { session } from "./session.svelte";

export type AddResult = { ok: true; host: string; warning?: string } | { ok: false; error: string };

class AccountsStore {
  accounts = $state.raw<Account[]>([]);
  loading = $state(false);
  loadError = $state<string>();
  private reads = new Resource<Account[]>({
    busy: (on) => (this.loading = on),
    apply: (accounts) => {
      this.accounts = accounts;
      this.loadError = undefined;
    },
    failed: (e) => (this.loadError = errorText(e)),
  });

  constructor() {
    session.onConnect((backend) => {
      backend.onCredentialsNeeded((host) => void this.restore(backend, host));
      // A change of the accounts, by this window or another.
      backend.onAccountsChanged(() => void this.load());
      void this.restoreAll(backend);
    });
    // A backend the host started again holds no tokens.
    session.onResync((backend) => void this.restoreAll(backend));
  }

  /**
   * Subscribes the connection to the backend's events, then hands it every token the keychain holds; each that it
   * did not hold is announced, which reads the accounts.
   */
  private async restoreAll(backend: Backend) {
    await this.load();
    try {
      const stored = await storedCredentials();
      for (const { host, token } of stored) await backend.setCredentials(host, token, false);
    } catch (e) {
      notices.failed("Cannot restore the saved access tokens", e);
    }
  }

  /** Answers a call for `host`'s credential with the keychain's token, if there is one; the backend announces it. */
  private async restore(backend: Backend, host: string) {
    try {
      const token = await storedCredential(host);
      if (token !== null) await backend.setCredentials(host, token, false);
    } catch (e) {
      notices.failed(`Cannot read the access token of ${host}`, e);
    }
  }

  /**
   * Asks the backend for the accounts, which also has it look each one up at its platform. The latest
   * reading counts; it resolves when the accounts are current.
   */
  async load(): Promise<void> {
    const backend = session.backend;
    if (backend) await this.reads.load(() => backend.accounts());
  }

  /**
   * Has the platform check `token` and keeps it if it is good. A token the platform refuses is `ok: false`
   * with the message to show; one that works but that the keychain would not keep is `ok` with a warning.
   */
  async addToken(host: string, token: string): Promise<AddResult> {
    const backend = session.backend;
    if (!backend) return { ok: false, error: "not connected to the backend" };
    let warning: string | undefined;
    let account: Account;
    try {
      account = await backend.setCredentials(host, token, true);
      try {
        await storeCredential(account.host, token);
      } catch (e) {
        warning = `The token works, but the keychain did not keep it, so it lasts until Vet quits: ${errorText(e)}`;
      }
    } catch (e) {
      return { ok: false, error: errorText(e) };
    }
    await this.load();
    return { ok: true, host: account.host, warning };
  }

  /**
   * Asks the platforms about the credentials again. A saved token is sent to the platform once more,
   * since a refusal would otherwise stand until the token changes. Resolves with the platform's message
   * if it still does not accept it.
   */
  async recheck(host: string): Promise<string | undefined> {
    const backend = session.backend;
    let message: string | undefined;
    try {
      const token = backend ? await storedCredential(host) : null;
      if (backend && token !== null) await backend.setCredentials(host, token, true);
    } catch (e) {
      message = errorText(e);
    }
    await this.load();
    return message;
  }

  /** Forgets the personal access token of `host`. Resolves with a message if that failed. */
  async remove(host: string): Promise<string | undefined> {
    const backend = session.backend;
    if (!backend) return "not connected to the backend";
    try {
      const accounts = await backend.removeCredentials(host);
      this.reads.cancel();
      this.accounts = accounts;
      await removeStoredCredential(host);
      return undefined;
    } catch (e) {
      return errorText(e);
    }
  }
}

export const accounts = new AccountsStore();

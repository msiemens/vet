// What the "Accounts" page shows per credential, derived from `Accounts`.

import type { Account } from "../../generated/Account";
import type { PlatformKind } from "../../generated/PlatformKind";

export type AccountStatus = "signedIn" | "rejected" | "unverified";

export interface AccountRow {
  /** One row per host and source. */
  key: string;
  host: string;
  /** The badge: GH, GHE for a GitHub Enterprise host, ADO. */
  platform: string;
  user?: string;
  /** The table's short name: "gh CLI", "az CLI" or "Token". */
  credential: string;
  /** Where the credential lives, for the details. */
  storage: string;
  status: AccountStatus;
  statusLabel: string;
  /** What the platform said when it did not accept the credential or could not be asked. */
  problem?: string;
  /** Why this credential matters or does not: a CLI's token is never stored, a token behind a CLI is not used. */
  note?: string;
  /** Only a personal access token is Vet's to remove or replace. */
  removable: boolean;
}

const CLI_NAMES: Record<PlatformKind, string> = { github: "gh CLI", azureDevOps: "az CLI" };

const STATUS_LABELS: Record<AccountStatus, string> = {
  signedIn: "Signed in",
  rejected: "Rejected",
  unverified: "Not verified",
};

export const GITHUB_HOST = "github.com";

export const cliName = (kind: PlatformKind): string => CLI_NAMES[kind];

function badge(account: Account): string {
  if (account.kind === "azureDevOps") return "ADO";
  return account.host === GITHUB_HOST ? "GH" : "GHE";
}

/** `keychain`: whether the host keeps personal access tokens in the system keychain. */
export function accountRow(account: Account, keychain: boolean): AccountRow {
  const { state } = account;
  const cli = account.source === "cli";
  let note: string | undefined;
  if (!account.inUse) note = "Not used: the signed-in CLI for this host comes first.";
  else if (cli) note = "Vet asks the CLI for a token when it needs one and never stores it.";
  return {
    key: `${account.host}/${account.source}`,
    host: account.host,
    platform: badge(account),
    user: account.user ?? undefined,
    credential: cli ? CLI_NAMES[account.kind] : "Token",
    storage: cli ? CLI_NAMES[account.kind] : keychain ? "Token in Keychain" : "Token, kept until Vet quits",
    status: state.type,
    statusLabel: STATUS_LABELS[state.type],
    problem: state.type === "signedIn" ? undefined : state.message,
    note,
    removable: !cli,
  };
}

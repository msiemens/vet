// What the "Accounts" page shows per credential, derived from `Accounts`.

import type { Account } from "../../generated/Account";
import type { PlatformKind } from "../../generated/PlatformKind";
import type { IconName } from "../../lib/ui/icons";

export type AccountStatus = "signedIn" | "rejected" | "unverified" | "unusable";

export interface AccountRow {
  /** One row per host and source. */
  key: string;
  host: string;
  /** The platform's mark and name; an Enterprise host shows GitHub's, its host column tells it apart. */
  platform: { icon: IconName; name: string };
  user?: string;
  /** The table's short name: "gh CLI", "az CLI" or "Token". */
  credential: string;
  /** Where the credential lives, for the details. */
  storage: string;
  status: AccountStatus;
  statusLabel: string;
  /** What the platform said when it did not accept the credential or could not be asked, or the CLI when it could not deliver its token. */
  problem?: string;
  /** Why this credential is not used: a token behind a signed-in CLI. */
  note?: string;
  /** Only a personal access token is Vet's to remove or replace. */
  removable: boolean;
  /** When the token stops working, where the platform says: "Expires on 3 Nov 2026" or "Expired on …". */
  expiry?: { label: string; soon: boolean };
}

/** A token that ends within this many days is pointed out. */
const EXPIRY_WARNING_DAYS = 7;

function expiry(expiresAt: number | null, now: number): AccountRow["expiry"] {
  if (expiresAt === null) return undefined;
  const date = new Date(expiresAt * 1000).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
  const past = expiresAt * 1000 <= now;
  return {
    label: `${past ? "Expired" : "Expires"} on ${date}`,
    soon: expiresAt * 1000 - now < EXPIRY_WARNING_DAYS * 24 * 3600 * 1000,
  };
}

const CLI_NAMES: Record<PlatformKind, string> = { github: "gh CLI", azureDevOps: "az CLI" };

const STATUS_LABELS: Record<AccountStatus, string> = {
  signedIn: "Signed in",
  rejected: "Rejected",
  unverified: "Not verified",
  unusable: "Unusable",
};

export const GITHUB_HOST = "github.com";

export const cliName = (kind: PlatformKind): string => CLI_NAMES[kind];

const PLATFORM_MARKS: Record<PlatformKind, { icon: IconName; name: string }> = {
  github: { icon: "github", name: "GitHub" },
  azureDevOps: { icon: "azureDevOps", name: "Azure DevOps" },
};

/** `keychain`: whether the host keeps personal access tokens in the system keychain; `now` in milliseconds. */
export function accountRow(account: Account, keychain: boolean, now = Date.now()): AccountRow {
  const { state } = account;
  const cli = account.source === "cli";
  const note =
    account.inUse || state.type === "unusable" ? undefined : "Not used: the signed-in CLI for this host comes first.";
  return {
    key: `${account.host}/${account.source}`,
    host: account.host,
    platform: PLATFORM_MARKS[account.kind],
    user: account.user ?? undefined,
    credential: cli ? CLI_NAMES[account.kind] : "Token",
    storage: cli ? CLI_NAMES[account.kind] : keychain ? "Token in Keychain" : "Token, kept until Vet quits",
    status: state.type,
    statusLabel: STATUS_LABELS[state.type],
    problem:
      state.type === "signedIn"
        ? undefined
        : state.type === "unusable"
          ? `As reported by ${CLI_NAMES[account.kind]}: ${state.message}`
          : state.message,
    note,
    removable: !cli,
    expiry: expiry(account.expiresAt, now),
  };
}

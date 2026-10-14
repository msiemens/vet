// What a request rejects with when it did not go through: the backend refused it (with the kind it
// names), or the backend could not be reached. What to do about a failure is decided from the kind,
// never from the message, which is only for showing.

import type { ErrorKind } from "../generated/ErrorKind";

export class RequestError extends Error {
  readonly kind: ErrorKind;

  constructor(kind: ErrorKind, message: string) {
    super(message);
    this.name = "RequestError";
    this.kind = kind;
  }
}

/** The kind of `error`, if it is a failed request. */
export const kindOf = (error: unknown): ErrorKind | undefined =>
  error instanceof RequestError ? error.kind : undefined;

/** Whether `error` is a failure of that kind. */
export const isKind = (error: unknown, kind: ErrorKind): boolean => kindOf(error) === kind;

/** What a request made against a revision the changeset moved on from rejects with. */
export const outdated = (): RequestError => new RequestError("outdated", "stale revision");

// What a request rejects with when it was given up on: not a failure, so nothing tells the viewer.

export function abortError(): DOMException {
  return new DOMException("The request was cancelled", "AbortError");
}

export function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

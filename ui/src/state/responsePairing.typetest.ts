// Compile-time test of the generated request/response pairing (A16). `pnpm check` compiles this
// file; it is never run, so it only carries type-level assertions. `pnpm test` cannot see it:
// the app tsconfig it is checked with (not the node one) is the one that ignores `*.test.ts`.
import type { ResponseOf } from "../generated/ResponseOf";

declare const threads: ResponseOf<"threads">;

// A `threads` answer is not the answer to a `manifest` request: the generated pairing types
// `backend.call` by the request, so reading one as the other is a type error.
// @ts-expect-error a threads answer is not the answer to a manifest request
export const notAManifest: ResponseOf<"manifest"> = threads;

declare const ping: ResponseOf<"ping">;

// The positive side: a `ping` is answered with a pong.
export const pong: { type: "pong"; payload: string } = ping;

declare const manifest: ResponseOf<"manifest">;

// And a `manifest` answer is the manifest, not just any response.
export const manifestRevision: number = manifest.revision;

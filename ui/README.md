# UI

Svelte + CodeMirror. The structure and its rules are in [ARCHITECTURE.md](ARCHITECTURE.md);
the desktop shell around it is in [src-tauri](src-tauri/README.md).

```sh
pnpm install
pnpm dev        # http://localhost:1420
pnpm check      # svelte-check
pnpm lint       # eslint, incl. the architecture boundaries (lint:fix)
pnpm format     # prettier (format:check in CI)
pnpm test       # unit tests
pnpm build
```

In a plain browser, pass the backend's address and the repository in the URL hash:
`http://localhost:1420/#port=<port>&token=<token>&repo=<path>`. Without `repo` the
start screen asks for a path. Start the backend with `cargo run -p vet-backend`; it
prints `{"port":…,"token":"…"}` on its first line.

The TS types in `src/generated` are generated from `vet-protocol`; after changing
it, run `cargo test -p vet-protocol`.

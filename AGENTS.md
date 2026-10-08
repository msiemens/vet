# Vet

Rules for working in this repository.

## Integration tests

A crate has at most one integration test binary, `tests/it/`. New integration tests are modules in it, never new files directly under `tests/`. Each file under `tests/` is a separate binary that links the whole dependency tree.

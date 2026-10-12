#!/usr/bin/env bash
# RUSTC_WRAPPER that lets sccache hit across checkouts of this repository. Plain
# `RUSTC_WRAPPER=sccache` misses for two reasons, both fixed here:
#
# 1. A crate whose source reads env!("OUT_DIR") (serde, octocrab, ...) gets the
#    checkout-specific build-script directory into sccache's key and into its
#    metadata, so it and everything depending on it misses. OUT_DIR is replaced by
#    a symlink with a stable name, derived from the build script's directory name
#    (crate name and hash). The link lives in /tmp, not $TMPDIR: $TMPDIR differs
#    per user, session and sandbox, and the path is part of the cache key.
# 2. sccache cannot cache proc-macro crates, and sccache hashes the proc-macro
#    library that dependents are compiled with. On macOS its linker output differs
#    per checkout because the install name is the absolute path of the file, so
#    proc-macros and dylibs get a fixed install name.
#
# Without sccache the compiler runs directly. If preparing the link fails, sccache
# runs without it.

if ! command -v sccache >/dev/null; then
    exec "$@"
fi

if [[ -n ${OUT_DIR:-} ]]; then
    links=/tmp/sccache-out-dir-$(id -u)
    link=$links/$(basename "$(dirname "$OUT_DIR")")
    if [[ $(uname) == Darwin ]]; then no_follow=-h; else no_follow=-T; fi
    # The rename is atomic, so concurrent builds never see a missing link.
    if mkdir -p "$links" && ln -s "$OUT_DIR" "$link.$$" 2>/dev/null; then
        if mv -f $no_follow "$link.$$" "$link" 2>/dev/null; then
            export OUT_DIR=$link
        else
            rm -f "$link.$$"
        fi
    fi
fi

if [[ $(uname) == Darwin ]]; then
    for arg in "$@"; do
        if [[ $arg == proc-macro || $arg == dylib ]]; then
            set -- "$@" -C link-arg=-Wl,-install_name,@rpath/lib.dylib
            break
        fi
    done
fi

exec sccache "$@"

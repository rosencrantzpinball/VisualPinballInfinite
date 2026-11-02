The program this site plays tables with (vpi.wasm) is Visual Pinball Infinite's player, GPL-3.0-or-later, built from
vpi-bf0a1b0a9e37.tar.gz here: tools/web/README.md in it says how.

Linked into it, each under its own licence:
- libwinevbs, Wine's VBScript engine (LGPL-2.1-or-later): https://github.com/vpinball/libwinevbs at the commit
  tools/web/libwinevbs.sh names, with the patches in tools/libwinevbs/patches and tools/web/patches;
- wasi-libc (its own licences, MIT and Apache-2.0 among them): https://github.com/WebAssembly/wasi-libc at the release
  tools/web/wasi-libc.sh names;
- the Rust crates Cargo.lock lists.

The table in table/ is its author's, under the author's terms.

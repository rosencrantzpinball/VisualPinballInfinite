#!/usr/bin/env bash
# Builds the toolchain CEF's Chromium is compiled with, from source, in the CEF build image (image.sh), into
# $CEF_WORK/toolchain/llvm:
#  - llvm: clang, lld and LLVM's tools from the LLVM commit Chromium 154 pins (fetch.sh fetches it), for arm64
#    alone, with compiler-rt's builtins and profile runtime (Chromium links the builtins); Debian's clang 19 builds
#    it. Chromium's newest code and its libc++ need this clang: Debian's is too old for them.
#  - gn: Chromium's build file generator, from the copy of its source in Chromium's tarball (Debian's clang will do).
#  - bindgen: Rust's binding generator, from the rust-bindgen commit Chromium's tools/rust/build_bindgen.py pins,
#    built with cargo and the image's Rust; it loads this LLVM's libclang when it runs.
#  - node: Node.js, the version Chromium's build checks for (third_party/node/update_node_binaries), from
#    nodejs.org's source tarball checked against its published SHA-256 sums, into $CEF_WORK/toolchain/node: the
#    tarball carries it prebuilt for x86-64 only.
#  - rust: Chromium's own Rust toolchain into $CEF_WORK/toolchain/rust, as Google builds it (tools/rust/build_rust.py:
#    Rust's x.py with tools/rust/config.toml.template's settings), from the Rust source the tarball carries
#    (third_party/rust-src, the revision Chromium pins) and against this LLVM, but bootstrapped by the image's Rust
#    1.98 (the source asks for a 1.98 beta) rather than a downloaded one. Chromium 154 needs its own toolchain rather
#    than any rustc: Blink uses crubit, whose bindings to Rust's standard library only it can make. Two stages, so
#    that rustc-dev (the compiler's crates, which crubit links) is the installed rustc's own.
#  - crubit: crubit's cc_bindings_from_rs, from the commit Chromium pins (tools/rust/update_rust.py), built with cargo
#    against that toolchain as tools/rust/build_crubit.py builds it, into the toolchain's bin.
#  - tsgo: TypeScript's native compiler, which DevTools' build runs (it has no other way on Linux), the version
#    Chromium's DEPS pins (third_party/typescript/README.chromium), from typescript-go's tag for it, into
#    $CEF_WORK/toolchain/tsgo; built by Go 1.26, itself built from go.dev's source (checked against go.dev's sums)
#    by Go 1.25, built the same way by the image's Debian Go 1.24. GOTOOLCHAIN=local: Go never fetches a toolchain.
#  - rollup: the native core of the Rollup DevTools bundles its front end with (its node_modules carry it for x86-64
#    alone), from Rollup's tag for the version there, built with cargo and Chromium's Rust toolchain built here, into
#    $CEF_WORK/toolchain/rollup (build.sh puts it beside Rollup, where Rollup looks first).
# Builds the parts named, or all of them in that order.
#
#   tools/steam-frame/cef/build-toolchain.sh [llvm] [gn] [bindgen] [node] [rust] [crubit] [tsgo] [rollup]
set -euo pipefail
source "$(dirname "$0")/env.sh"
PREFIX=$CEF_WORK/toolchain/llvm
RUST_PREFIX=$CEF_WORK/toolchain/rust
parts=("$@")
[[ ${#parts[@]} -gt 0 ]] || parts=(llvm gn bindgen node rust crubit tsgo rollup)
JOBS=${JOBS:-12}
echo "system disk: $(df -h / | awk 'NR==2 {print $4}') free; data disk: $(df -h "$WORK" | awk 'NR==2 {print $4}') free"

for part in "${parts[@]}"; do
   case $part in
   llvm)
      [[ -d $CEF_WORK/src/llvm-project/llvm ]] || { echo "no LLVM source (fetch.sh)" >&2; exit 1; }
      in_cef_image bash -c "
         set -euo pipefail
         cmake -G Ninja -S '$CEF_WORK/src/llvm-project/llvm' -B '$CEF_WORK/build/llvm' \
            -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_INSTALL_PREFIX='$PREFIX' \
            -DCMAKE_C_COMPILER=/usr/lib/llvm-19/bin/clang -DCMAKE_CXX_COMPILER=/usr/lib/llvm-19/bin/clang++ \
            -DLLVM_USE_LINKER=lld \
            -DLLVM_ENABLE_PROJECTS='clang;lld' \
            -DLLVM_ENABLE_RUNTIMES=compiler-rt \
            -DLLVM_TARGETS_TO_BUILD=AArch64 \
            -DLLVM_ENABLE_ASSERTIONS=OFF \
            -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_DOCS=OFF \
            -DCLANG_INCLUDE_TESTS=OFF -DCLANG_ENABLE_STATIC_ANALYZER=OFF -DCLANG_PLUGIN_SUPPORT=OFF \
            -DLLVM_ENABLE_BINDINGS=OFF -DLLVM_ENABLE_LIBXML2=OFF \
            -DLLVM_PARALLEL_LINK_JOBS=4 \
            -DCOMPILER_RT_DEFAULT_TARGET_ONLY=ON \
            -DCOMPILER_RT_BUILD_SANITIZERS=OFF -DCOMPILER_RT_BUILD_XRAY=OFF -DCOMPILER_RT_BUILD_LIBFUZZER=OFF \
            -DCOMPILER_RT_BUILD_MEMPROF=OFF -DCOMPILER_RT_BUILD_ORC=OFF -DCOMPILER_RT_BUILD_GWP_ASAN=OFF \
            -DCOMPILER_RT_BUILD_CTX_PROFILE=OFF -DCOMPILER_RT_BUILD_PROFILE=ON
         ninja -C '$CEF_WORK/build/llvm' -j $JOBS install
         '$PREFIX/bin/clang' --version | head -1
         ls '$PREFIX/lib/clang/$CLANG_VERSION/lib/'*
      "
      ;;
   gn)
      [[ -d $CHROMIUM_SRC/tools/gn ]] || { echo "no Chromium source (fetch.sh)" >&2; exit 1; }
      in_cef_image bash -c "
         set -euo pipefail
         cd '$CHROMIUM_SRC'
         CC=/usr/lib/llvm-19/bin/clang CXX=/usr/lib/llvm-19/bin/clang++ AR=/usr/lib/llvm-19/bin/llvm-ar \
            LDFLAGS=-fuse-ld=lld python3 tools/gn/bootstrap/bootstrap.py --skip-generate-buildfiles -j $JOBS
         install -D -m 755 out/Release/gn '$PREFIX/bin/gn'
         '$PREFIX/bin/gn' --version
      "
      ;;
   bindgen)
      [[ -f $CHROMIUM_SRC/tools/rust/build_bindgen.py ]] || { echo "no Chromium source (fetch.sh)" >&2; exit 1; }
      commit=$(grep -o -E "^BINDGEN_GIT_VERSION = '[0-9a-f]+'" "$CHROMIUM_SRC/tools/rust/build_bindgen.py" | cut -d"'" -f2 || true)
      [[ -n $commit ]] || { echo "Chromium's build_bindgen.py names no BINDGEN_GIT_VERSION" >&2; exit 1; }
      source_dir=$CEF_WORK/src/rust-bindgen
      if [[ ! -d $source_dir/.git ]] || [[ $(git -C "$source_dir" rev-parse HEAD) != "$commit" ]]; then
         mkdir -p "$source_dir"
         git -C "$source_dir" init -q
         git -C "$source_dir" remote add origin https://github.com/rust-lang/rust-bindgen.git 2>/dev/null || true
         git -C "$source_dir" fetch -q --depth 1 origin "$commit"
         git -C "$source_dir" checkout -q FETCH_HEAD
      fi
      in_cef_image bash -c "
         set -euo pipefail
         cargo build --locked --release --manifest-path '$source_dir/Cargo.toml' --target-dir '$CEF_WORK/build/bindgen' --bin bindgen
         install -D -m 755 '$CEF_WORK/build/bindgen/release/bindgen' '$PREFIX/bin/bindgen'
         LIBCLANG_PATH='$PREFIX/lib' '$PREFIX/bin/bindgen' --version
      "
      ;;
   node)
      version=$(grep -o -E '^NODE_VERSION="v[0-9.]+"' "$CHROMIUM_SRC/third_party/node/update_node_binaries" | cut -d'"' -f2 || true)
      [[ -n $version ]] || { echo "Chromium's update_node_binaries names no NODE_VERSION" >&2; exit 1; }
      tarball=node-$version.tar.xz
      sums=$CEF_WORK/dl/node-$version-SHASUMS256.txt
      [[ -f $sums ]] || curl -fsS -o "$sums" "https://nodejs.org/dist/$version/SHASUMS256.txt"
      want=$(awk -v f="$tarball" '$2 == f {print $1}' "$sums")
      [[ -n $want ]] || { echo "nodejs.org's sums name no $tarball" >&2; exit 1; }
      if [[ ! -f $CEF_WORK/dl/$tarball ]] || [[ $(shasum -a 256 "$CEF_WORK/dl/$tarball" | cut -d' ' -f1) != "$want" ]]; then
         curl -fsS --retry 5 -o "$CEF_WORK/dl/$tarball" "https://nodejs.org/dist/$version/$tarball"
         [[ $(shasum -a 256 "$CEF_WORK/dl/$tarball" | cut -d' ' -f1) == "$want" ]] || { echo "$tarball: sha256 differs from nodejs.org's" >&2; exit 1; }
      fi
      in_cef_image bash -c "
         set -euo pipefail
         rm -rf '$CEF_WORK/build/node-$version'
         tar -xJf '$CEF_WORK/dl/$tarball' -C '$CEF_WORK/build'
         cd '$CEF_WORK/build/node-$version'
         ./configure --ninja
         ninja -C out/Release -j $JOBS node
         install -D -m 755 out/Release/node '$CEF_WORK/toolchain/node/bin/node'
         '$CEF_WORK/toolchain/node/bin/node' --version
      "
      ;;
   rust)
      [[ -f $CHROMIUM_SRC/third_party/rust-src/x.py ]] || { echo "no Rust source in Chromium's tree (fetch.sh)" >&2; exit 1; }
      stage0=/opt/rust/rustup/toolchains/1.98-aarch64-unknown-linux-gnu
      config=$CEF_WORK/build/rust-config.toml
      mkdir -p "$CEF_WORK/build"
      cat > "$config" <<EOF
change-id = "ignore"
[llvm]
download-ci-llvm = false
static-libstdcpp = true
[rust]
download-rustc = false
channel = "nightly"
[build]
build = "aarch64-unknown-linux-gnu"
host = ["aarch64-unknown-linux-gnu"]
target = ["aarch64-unknown-linux-gnu"]
build-dir = "$CEF_WORK/build/rust"
rustc = "$stage0/bin/rustc"
cargo = "$stage0/bin/cargo"
locked-deps = true
docs = false
profiler = true
extended = true
tools = ["cargo", "rustc-dev", "rustdoc", "rustfmt", "src"]
jobs = ${JOBS_RUST:-10}
description = "chromium"
[install]
prefix = "$RUST_PREFIX"
sysconfdir = "etc"
[target.aarch64-unknown-linux-gnu]
llvm-config = "$PREFIX/bin/llvm-config"
ar = "$PREFIX/bin/llvm-ar"
cc = "$PREFIX/bin/clang"
cxx = "$PREFIX/bin/clang++"
linker = "$PREFIX/bin/clang"
EOF
      in_cef_image bash -c "
         set -euo pipefail
         cd '$CHROMIUM_SRC/third_party/rust-src'
         CARGO_HOME='$CEF_WORK/build/rust-cargo-home' python3 x.py install --config '$config'
         '$RUST_PREFIX/bin/rustc' -vV
      "
      ;;
   crubit)
      commit=$(grep -o -E "^CRUBIT_REVISION = '[0-9a-f]+'" "$CHROMIUM_SRC/tools/rust/update_rust.py" | cut -d"'" -f2 || true)
      [[ -n $commit ]] || { echo "Chromium's update_rust.py names no CRUBIT_REVISION" >&2; exit 1; }
      [[ -x $RUST_PREFIX/bin/rustc ]] || { echo "no Rust toolchain in $RUST_PREFIX (build-toolchain.sh rust)" >&2; exit 1; }
      source_dir=$CEF_WORK/src/crubit
      if [[ ! -d $source_dir/.git ]] || [[ $(git -C "$source_dir" rev-parse HEAD) != "$commit" ]]; then
         mkdir -p "$source_dir"
         git -C "$source_dir" init -q
         git -C "$source_dir" remote add origin https://github.com/google/crubit.git 2>/dev/null || true
         git -C "$source_dir" fetch -q --depth 1 origin "$commit"
         git -C "$source_dir" checkout -q FETCH_HEAD
      fi
      # the rpath lets it find the toolchain's librustc_driver, as rustc does
      in_cef_image bash -c "
         set -euo pipefail
         export PATH='$RUST_PREFIX/bin':\$PATH CARGO_HOME='$CEF_WORK/build/crubit/cargo_home'
         export CARGO_ENCODED_RUSTFLAGS=\$(printf '%s\x1f%s' '-Clink-args=-Wl,-z,origin' '-Clink-args=-Wl,-rpath,\$ORIGIN/../lib')
         cargo build --release --locked --bin cc_bindings_from_rs --target-dir '$CEF_WORK/build/crubit' \
            --manifest-path '$source_dir/cargo/cc_bindings_from_rs/cc_bindings_from_rs/Cargo.toml'
         install -m 755 '$CEF_WORK/build/crubit/release/cc_bindings_from_rs' '$RUST_PREFIX/bin/cc_bindings_from_rs'
         '$RUST_PREFIX/bin/cc_bindings_from_rs' --help | head -2
      "
      ;;
   tsgo)
      version=$(sed -n 's/^Version: //p' "$CHROMIUM_SRC/third_party/typescript/README.chromium")
      [[ -n $version ]] || { echo "Chromium's third_party/typescript names no version" >&2; exit 1; }
      [[ -f $CEF_WORK/dl/go-releases.json ]] || curl -fsS -o "$CEF_WORK/dl/go-releases.json" "https://go.dev/dl/?mode=json&include=all"
      for go in $GO_BOOTSTRAP $GO; do
         tarball=$go.src.tar.gz
         want=$(python3 -I -c 'import json, sys; print(next(f["sha256"] for r in json.load(open(sys.argv[1])) for f in r["files"] if f["filename"] == sys.argv[2]))' "$CEF_WORK/dl/go-releases.json" "$tarball")
         if [[ ! -f $CEF_WORK/dl/$tarball ]] || [[ $(shasum -a 256 "$CEF_WORK/dl/$tarball" | cut -d' ' -f1) != "$want" ]]; then
            curl -fsSL -o "$CEF_WORK/dl/$tarball" "https://go.dev/dl/$tarball"
            [[ $(shasum -a 256 "$CEF_WORK/dl/$tarball" | cut -d' ' -f1) == "$want" ]] || { echo "$tarball: sha256 differs from go.dev's" >&2; exit 1; }
         fi
      done
      source_dir=$CEF_WORK/src/typescript-go
      tag=typescript/v$version
      if [[ ! -d $source_dir/.git ]] || ! git -C "$source_dir" describe --tags --exact-match HEAD 2>/dev/null | grep -qx "$tag"; then
         mkdir -p "$source_dir"
         git -C "$source_dir" init -q
         git -C "$source_dir" remote add origin https://github.com/microsoft/typescript-go.git 2>/dev/null || true
         git -C "$source_dir" fetch -q --depth 1 origin "refs/tags/$tag:refs/tags/$tag"
         git -C "$source_dir" checkout -q "$tag"
      fi
      in_cef_image bash -c "
         set -euo pipefail
         export GOTOOLCHAIN=local GOPATH='$CEF_WORK/build/gopath' GOCACHE='$CEF_WORK/build/gocache'
         for pair in '$GO_BOOTSTRAP=/usr/lib/go-1.24' '$GO=$CEF_WORK/toolchain/$GO_BOOTSTRAP'; do
            go=\${pair%%=*}
            if [[ ! -x '$CEF_WORK/toolchain/'\$go/bin/go ]]; then
               rm -rf '$CEF_WORK/toolchain/'\$go '$CEF_WORK/build/'\$go
               mkdir -p '$CEF_WORK/build/'\$go
               tar -xzf '$CEF_WORK/dl/'\$go.src.tar.gz -C '$CEF_WORK/build/'\$go
               (cd '$CEF_WORK/build/'\$go/go/src && GOROOT_BOOTSTRAP=\${pair#*=} ./make.bash)
               mv '$CEF_WORK/build/'\$go/go '$CEF_WORK/toolchain/'\$go
            fi
         done
         cd '$source_dir'
         # as typescript-go's own build makes it (Herebyfile.mjs): its library files read from beside it (noembed),
         # where Chromium declares them, rather than built in
         CGO_ENABLED=0 '$CEF_WORK/toolchain/$GO/bin/go' build -trimpath -ldflags='-s -w' -tags=noembed \
            -o '$CEF_WORK/toolchain/tsgo/tsc' ./cmd/tsgo
         cp '$CHROMIUM_SRC/third_party/typescript/linux-amd64/src/lib/'*.d.ts '$CEF_WORK/toolchain/tsgo/'
         '$CEF_WORK/toolchain/tsgo/tsc' --version
      "
      ;;
   rollup)
      version=$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' \
         "$CHROMIUM_SRC/third_party/devtools-frontend/src/node_modules/rollup/package.json")
      [[ -x $RUST_PREFIX/bin/cargo ]] || { echo "no Rust toolchain in $RUST_PREFIX (build-toolchain.sh rust)" >&2; exit 1; }
      source_dir=$CEF_WORK/src/rollup
      if [[ ! -d $source_dir/.git ]] || ! git -C "$source_dir" describe --tags --exact-match HEAD 2>/dev/null | grep -qx "v$version"; then
         mkdir -p "$source_dir"
         git -C "$source_dir" init -q
         git -C "$source_dir" remote add origin https://github.com/rollup/rollup.git 2>/dev/null || true
         git -C "$source_dir" fetch -q --depth 1 origin "refs/tags/v$version:refs/tags/v$version"
         git -C "$source_dir" checkout -q "v$version"
      fi
      # our toolchain's own cargo, not rustup's: Rollup's rust-toolchain.toml would have rustup fetch a nightly
      in_cef_image bash -c "
         set -euo pipefail
         export PATH='$RUST_PREFIX/bin':\$PATH CARGO_HOME='$CEF_WORK/build/rollup/cargo_home'
         cargo build --release --locked -p bindings_napi --manifest-path '$source_dir/rust/Cargo.toml' \
            --target-dir '$CEF_WORK/build/rollup'
         install -D -m 755 '$CEF_WORK/build/rollup/release/libbindings_napi.so' \
            '$CEF_WORK/toolchain/rollup/rollup.linux-arm64-gnu.node'
         file '$CEF_WORK/toolchain/rollup/rollup.linux-arm64-gnu.node' | cut -c1-120
      "
      ;;
   *) echo "unknown part: $part (llvm, gn, bindgen, node, rust, crubit, tsgo, rollup)" >&2; exit 1 ;;
   esac
done
echo "system disk: $(df -h / | awk 'NR==2 {print $4}') free; data disk: $(df -h "$WORK" | awk 'NR==2 {print $4}') free"

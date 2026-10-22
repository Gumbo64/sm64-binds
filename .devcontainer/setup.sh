#!/usr/bin/env bash
#
# Toolchain setup for the parts that devcontainer features do not cover.
# Invoked automatically by devcontainer.json (postCreateCommand) and safe to
# re-run: every step is skipped when the tool is already installed.
#
#   emcc / em++      -> `make sm64-build` (builds sm64-port with TARGET_WEB=1)
#   wasm-pack        -> `wasm-pack build` in rust/ (uses the wasm-opt settings
#                       in rust/Cargo.toml)
#   wasm-bindgen     -> wasm-bindgen CLI, version pinned to rust/Cargo.lock
#   binaryen         -> wasm-opt, the optimizer wasm-pack calls in release mode
#
# Versions can be overridden from the environment, e.g.
#   EMSDK_VERSION=6.0.13 devcontainer up --workspace-folder .
# EMSDK_VERSION defaults to "latest": the newest emscripten release, looked up
# on every run (so a rebuild can pick up a new compiler; pin it to reproduce).
#
set -euo pipefail

EMSDK_DIR="${EMSDK_DIR:-/usr/local/emsdk}"
BIN_DIR="${BIN_DIR:-/usr/local/bin}"
EMSDK_VERSION="${EMSDK_VERSION:-latest}"
WASM_PACK_VERSION="${WASM_PACK_VERSION:-0.15.0}"
# Keep in sync with the wasm-bindgen entry in rust/Cargo.lock
WASM_BINDGEN_VERSION="${WASM_BINDGEN_VERSION:-0.2.108}"
# Binaryen release bundled with the wasm-pack version above. wasm-pack compares
# this against the system wasm-opt and prompts interactively on a mismatch.
BINARYEN_VERSION="${BINARYEN_VERSION:-117}"

WORKSPACE="${1:-$PWD}"

# Logs go to stderr so that "$(fetch_tarball ...)" only captures the path.
log() { printf '\n\033[1;34m[setup]\033[0m %s\n' "$*" >&2; }
die() { printf '\n\033[1;31m[setup] %s\033[0m\n' "$*" >&2; exit 1; }

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
    SUDO="sudo"
fi

case "$(uname -m)" in
    x86_64 | amd64)  ARCH="x86_64" ;;
    aarch64 | arm64) ARCH="aarch64" ;;
    *) die "unsupported architecture: $(uname -m)" ;;
esac

# Prints the first line of a version command without risking a SIGPIPE for the
# command itself (which would trip `set -o pipefail`).
first_line() { "$@" 2>/dev/null | sed -n '1p' || true; }

# Downloads a .tar.gz into a temp dir and echoes the path of the directory that
# was extracted inside it.
fetch_tarball() {
    local url="$1" inner="$2" tmp
    tmp="$(mktemp -d)"
    log "downloading ${url##*/}"
    curl -fsSL --proto '=https' --tlsv1.2 "$url" | tar -xz -C "$tmp"
    printf '%s' "${tmp}/${inner}"
}

# ---------------------------------------------------------------------------
# 1. Host build tools
# ---------------------------------------------------------------------------
# mcr.microsoft.com/devcontainers/base:ubuntu is derived from buildpack-deps and
# already ships gcc/make/python3/git, so this normally does nothing. It is kept
# as a safety net in case the base image changes.
missing=()
for tool in gcc g++ make python3 git curl; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ ${#missing[@]} -gt 0 ]; then
    log "installing build tools: ${missing[*]}"
    ${SUDO} apt-get update -y
    ${SUDO} apt-get install -y --no-install-recommends "${missing[@]}"
    ${SUDO} rm -rf /var/lib/apt/lists/*
fi

# ---------------------------------------------------------------------------
# 2. Emscripten SDK (emcc) - builds the WASM that rust/pkg/ ships
# ---------------------------------------------------------------------------
installed_emscripten_version() {
    local version_file="${EMSDK_DIR}/upstream/emscripten/emscripten-version.txt"
    if [ -f "$version_file" ]; then
        tr -d ' "[:space:]' <"$version_file"
    fi
}

# emsdk itself: cloned once, then updated, so that its list of releases (and
# what "latest" means) is current.
if [ -d "${EMSDK_DIR}/.git" ]; then
    log "updating emsdk in ${EMSDK_DIR}"
    git -C "$EMSDK_DIR" pull --ff-only --quiet || log "  could not update emsdk; using its current release list"
else
    log "cloning emsdk into ${EMSDK_DIR}"
    ${SUDO} rm -rf "$EMSDK_DIR"
    ${SUDO} git clone --depth 1 https://github.com/emscripten-core/emsdk.git "$EMSDK_DIR"
    # The emscripten cache (sysroot, ports, wasm-ld) lives inside the emsdk
    # directory, so the whole tree has to stay writable for whoever builds.
    ${SUDO} chown -R "$(id -u):$(id -g)" "$EMSDK_DIR"
fi

# "latest" (and the other aliases) to a version number, from emsdk's own table.
wanted="$(python3 -c 'import json, sys
tags = json.load(open(sys.argv[1]))
v = sys.argv[2]
while v in tags["aliases"]:
    v = tags["aliases"][v]
print(v)' "${EMSDK_DIR}/emscripten-releases-tags.json" "$EMSDK_VERSION")"

if [ "$(installed_emscripten_version)" = "$wanted" ]; then
    log "emscripten ${wanted} already installed"
else
    log "installing emscripten ${wanted} (${ARCH}; was: $(installed_emscripten_version || true))"
    "$EMSDK_DIR/emsdk" install "$wanted"
    "$EMSDK_DIR/emsdk" activate "$wanted"
fi

# Emscripten's entry point scripts exec "$0.py", so they cannot be symlinked
# into /usr/local/bin. These tiny wrappers put emcc on the default PATH without
# overriding the PATH that the devcontainer features contribute.
for tool in emcc em++ emar emcmake emconfigure emmake emnm emranlib emsize emstrip; do
    target="${EMSDK_DIR}/upstream/emscripten/${tool}"
    [ -f "$target" ] || continue
    ${SUDO} tee "${BIN_DIR}/${tool}" >/dev/null <<EOF
#!/bin/sh
# Generated by .devcontainer/setup.sh - do not edit.
export EM_CONFIG="\${EM_CONFIG:-${EMSDK_DIR}/.emscripten}"
exec "${target}" "\$@"
EOF
    ${SUDO} chmod 0755 "${BIN_DIR}/${tool}"
done

# ---------------------------------------------------------------------------
# 3. wasm tooling (wasm-pack release builds of rust/)
# ---------------------------------------------------------------------------
if ! command -v wasm-pack >/dev/null 2>&1; then
    log "installing wasm-pack ${WASM_PACK_VERSION}"
    dir="wasm-pack-v${WASM_PACK_VERSION}-${ARCH}-unknown-linux-musl"
    tmp="$(fetch_tarball \
        "https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/${dir}.tar.gz" \
        "$dir")"
    ${SUDO} install -m 0755 "${tmp}/wasm-pack" ${BIN_DIR}/wasm-pack
    rm -rf "$(dirname "$tmp")"
else
    log "wasm-pack already installed"
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
    log "installing wasm-bindgen ${WASM_BINDGEN_VERSION} (matches rust/Cargo.lock)"
    dir="wasm-bindgen-${WASM_BINDGEN_VERSION}-${ARCH}-unknown-linux-musl"
    tmp="$(fetch_tarball \
        "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WASM_BINDGEN_VERSION}/${dir}.tar.gz" \
        "$dir")"
    ${SUDO} install -m 0755 "${tmp}/wasm-bindgen" ${BIN_DIR}/wasm-bindgen
    rm -rf "$(dirname "$tmp")"
else
    log "wasm-bindgen already installed"
fi

if ! command -v wasm-opt >/dev/null 2>&1; then
    log "installing binaryen ${BINARYEN_VERSION} (wasm-opt)"
    dir="binaryen-version_${BINARYEN_VERSION}"
    tmp="$(fetch_tarball \
        "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/${dir}-${ARCH}-linux.tar.gz" \
        "$dir")"
    ${SUDO} cp -r "${tmp}/bin/." ${BIN_DIR}/
    rm -rf "$(dirname "$tmp")"
else
    log "wasm-opt already installed"
fi

# LLVM 18, which wasmer's LLVM backend links against: `make sm64-build` uses it
# to precompile the game for the Rust crate (maketools/precompile.py). Only
# needed to make rust/pkg/*.wasmu, not to use the crate.
if [ ! -x /usr/lib/llvm-18/bin/llvm-config ]; then
    log "installing LLVM 18 (for maketools/precompile)"
    ${SUDO} apt-get update -y
    ${SUDO} apt-get install -y --no-install-recommends \
        llvm-18-dev libpolly-18-dev zlib1g-dev libzstd-dev libxml2-dev
    ${SUDO} rm -rf /var/lib/apt/lists/*
else
    log "LLVM 18 already installed"
fi

# ---------------------------------------------------------------------------
# 4. Report
# ---------------------------------------------------------------------------
log "toolchain"
if command -v cargo >/dev/null 2>&1; then
    log "  rustc:         $(first_line rustc --version)"
    log "  cargo:         $(first_line cargo --version)"
    log "  targets:       $(rustup target list --installed 2>/dev/null | tr '\n' ' ' || true)"
    if ! rustup target list --installed 2>/dev/null | grep -qx 'wasm32-unknown-unknown'; then
        log "  WARNING: the wasm32-unknown-unknown target is missing (rustup target add wasm32-unknown-unknown)"
    fi
else
    log "  WARNING: cargo is not on PATH - was the rust devcontainer feature applied?"
fi
log "  python3:       $(first_line python3 --version)"
log "  make:          $(first_line make --version)"
log "  emcc:          $(first_line emcc --version)"
log "  wasm-pack:     $(first_line wasm-pack --version)"
log "  wasm-bindgen:  $(first_line wasm-bindgen --version)"
log "  wasm-opt:      $(first_line wasm-opt --version)"

# rust/src/lib.rs embeds the ROM with include_bytes!(), so neither the crate nor
# its tests compile without it.
if [ -f "${WORKSPACE}/baserom.us.z64" ]; then
    log "baserom.us.z64 found - 'cargo test' in rust/ will run"
else
    log "WARNING: ${WORKSPACE}/baserom.us.z64 is missing."
    log "         A US .z64 ROM (8.00MB) is required to build the rust crate"
    log "         and its tests, because it is embedded with include_bytes!()."
fi

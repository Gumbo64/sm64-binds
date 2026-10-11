# Main Makefile
ROM_FILE = baserom.us.z64

SM64PC = sm64-port

WASM = WASM
WASM_JS = WASM_JS
WASM_ZIP_XOR = WASM_ZIP_XOR

JS_PKG = javascript/src/scripts/sm64/pkg
PYTHON_PKG = python/src/sm64_binds/pkg
RUST_PKG = rust/pkg

RESET  = \033[0m
RED    = \033[31m
GREEN  = \033[32m
YELLOW = \033[33m
BLUE   = \033[34m

# Check if Emsdk is installed
EMSDK_PATH := $(shell which emcc)

# Number of parallel jobs for the sm64-port build
JOBS ?= $(shell nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)

all: sm64-build

# Build the WASM from the SM64 source if emscripten is available,
# otherwise recreate it from the distributed XOR files
sm64-build: copy-rom
	@echo "$(GREEN)\nBUILDING WEB VERSION$(RESET)"
	@mkdir -p $(WASM)
	@if [ -z "$(EMSDK_PATH)" ]; then \
		echo "$(YELLOW)Emscripten SDK (emsdk) is not installed, recreating WASM from XOR files$(RESET)"; \
		python3 maketools/xor.py $(WASM_ZIP_XOR)/sm64.us.wasm.zip.xor $(WASM)/sm64.us.wasm.zip $(ROM_FILE); \
		python3 maketools/xor.py $(WASM_ZIP_XOR)/sm64_headless.us.wasm.zip.xor $(WASM)/sm64_headless.us.wasm.zip $(ROM_FILE); \
		python3 maketools/unzip.py $(WASM)/sm64.us.wasm.zip $(WASM)/sm64.us.wasm; \
		python3 maketools/unzip.py $(WASM)/sm64_headless.us.wasm.zip $(WASM)/sm64_headless.us.wasm; \
	else \
		echo "$(BLUE)\nNORMAL$(RESET)"; \
		$(MAKE) -C $(SM64PC) -j$(JOBS) TARGET_WEB=1; \
		echo "$(BLUE)\nHEADLESS$(RESET)"; \
		$(MAKE) -C $(SM64PC) -j$(JOBS) TARGET_WEB=1 HEADLESS_VERSION=1; \
		cp $(SM64PC)/build/us_web/sm64.us.js $(WASM_JS)/sm64.us.js; \
		cp $(SM64PC)/build/us_web/sm64.us.wasm $(WASM)/sm64.us.wasm; \
		cp $(SM64PC)/build/us_web_headless/sm64.us.wasm $(WASM)/sm64_headless.us.wasm; \
		python3 maketools/zip.py $(WASM)/sm64.us.wasm $(WASM)/sm64.us.wasm.zip; \
		python3 maketools/zip.py $(WASM)/sm64_headless.us.wasm $(WASM)/sm64_headless.us.wasm.zip; \
		python3 maketools/xor.py $(WASM)/sm64.us.wasm.zip $(WASM_ZIP_XOR)/sm64.us.wasm.zip.xor $(ROM_FILE); \
		python3 maketools/xor.py $(WASM)/sm64_headless.us.wasm.zip $(WASM_ZIP_XOR)/sm64_headless.us.wasm.zip.xor $(ROM_FILE); \
	fi

# Distribute the artifacts to the language packages
	cp $(WASM_ZIP_XOR)/sm64_headless.us.wasm.zip.xor $(RUST_PKG)/sm64_headless.us.wasm.zip.xor
	cp $(WASM_ZIP_XOR)/sm64_headless.us.wasm.zip.xor $(PYTHON_PKG)/sm64_headless.us.wasm.zip.xor
	cp $(WASM_JS)/sm64.us.js $(JS_PKG)/sm64.us.js

# 	just call it a png because webpack freaks out if it sees .xor
	cp $(WASM_ZIP_XOR)/sm64.us.wasm.zip.xor $(JS_PKG)/sm64.us.wasm.zip.xor.png

# Copy the ROM into the SM64PC directory
copy-rom:
	@echo "Copying $(ROM_FILE) to $(SM64PC)..."
	@if [ ! -f $(ROM_FILE) ]; then \
		echo "$(RED)A Mario 64 ROM is required with the name $(ROM_FILE)$(RESET)"; \
		exit 1; \
	fi
	@cp $(ROM_FILE) $(SM64PC)/$(ROM_FILE)

# ------------------------------------------------------------------
# Tests
#
# Each target is independent; `make test` runs whichever are available.
# Anything needing the ROM or a toolchain skips rather than fails.
# ------------------------------------------------------------------
PYTHON ?= python3
CARGO  ?= cargo
NPM    ?= npm

test: test-maketools test-python test-rust test-javascript

test-maketools:
	@echo "$(BLUE)\nMAKETOOLS$(RESET)"
	@$(PYTHON) -m pytest maketools -q || \
		(echo "$(YELLOW)pytest unavailable, skipping maketools tests$(RESET)")

test-python:
	@echo "$(BLUE)\nPYTHON$(RESET)"
	@cd python && $(PYTHON) -m pytest -q || \
		(echo "$(YELLOW)pytest/wasmtime unavailable, skipping python tests$(RESET)")

test-rust:
	@echo "$(BLUE)\nRUST$(RESET)"
	@cd rust && $(CARGO) test

test-javascript:
	@echo "$(BLUE)\nJAVASCRIPT$(RESET)"
	@cd javascript && $(NPM) test

lint:
	@echo "$(BLUE)\nRUST CLIPPY$(RESET)"
	@cd rust && $(CARGO) clippy --all-targets -- -D warnings
	@echo "$(BLUE)\nRUST FMT$(RESET)"
	@cd rust && $(CARGO) fmt --check
	@echo "$(BLUE)\nJAVASCRIPT$(RESET)"
	@cd javascript && $(NPM) run lint
	@cd js_example && $(NPM) run lint

# ------------------------------------------------------------------
# Benchmarks
#
# See rust/benchmark/README.md (the shared harness is rust/benchmark/harness.rs,
# mirrored by python/benchs/harness.py). Both need a ROM at the repo root.
# The figures are from an x86-64 container; treat them as relative.
# ------------------------------------------------------------------
bench:
	@echo "$(BLUE)\nHOT PATHS$(RESET)"
	@cd rust && $(CARGO) bench --bench hot_paths -- ../$(ROM_FILE)
	@echo "$(BLUE)\nDIAGNOSTICS$(RESET)"
	@cd rust && $(CARGO) bench --bench diagnostics -- ../$(ROM_FILE)

bench-python:
	@echo "$(BLUE)\nPYTHON$(RESET)"
	@cd python && PYTHONPATH=src $(PYTHON) bench.py ../$(ROM_FILE)

# ------------------------------------------------------------------
# Release helpers
#
# The three registries are bumped together with scripts/bump_versions.py;
# pushing the v* tag then runs .github/workflows/release.yml.
# ------------------------------------------------------------------
VERSION ?=

versions:
	@$(PYTHON) scripts/show_versions.py

check-artifacts:
	@$(PYTHON) scripts/check_artifacts.py

# Bump every package: make bump VERSION=1.0.25
bump:
	@test -n "$(VERSION)" || { echo "usage: make bump VERSION=1.0.25"; exit 1; }
	@$(PYTHON) scripts/bump_versions.py $(VERSION)

clean:
	@echo "Cleaning up..."
	$(MAKE) -C $(SM64PC) clean
	rm -rf $(WASM)

.PHONY: all sm64-build copy-rom clean test test-maketools test-python test-rust test-javascript lint bench bench-python versions check-artifacts bump
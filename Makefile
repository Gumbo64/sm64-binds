# Main Makefile
ROM_FILE = baserom.us.z64

SM64PC = sm64-port

WASM = WASM
WASM_JS = WASM_JS
WASM_ZIP_XOR = WASM_ZIP_XOR

JS_PKG = javascript/src/scripts/sm64/pkg
PYTHON_PKG = python/pkg
RUST_PKG = rust/pkg

RUST = rust

RESET  = \033[0m
RED    = \033[31m
GREEN  = \033[32m
YELLOW = \033[33m
BLUE   = \033[34m

# Define the default target that will build both web and CLI versions
all: cli-build sm64-build

# Check if Emsdk is installed
EMSDK_PATH := $(shell which emcc)

sm64-build: copy-rom
	@echo "$(GREEN)\nBUILDING WEB VERSION$(RESET)"	
	@if [ -z "$(EMSDK_PATH)" ]; then \
		echo "$(YELLOW)Emscripten SDK (emsdk) is not installed, recreating WASM from XOR files $(RESET)"; \
		python3 maketools/xor.py $(WASM_ZIP_XOR)/sm64.us.wasm.zip.xor $(WASM)/sm64.us.wasm.zip  $(ROM_FILE); \
		python3 maketools/xor.py $(WASM_ZIP_XOR)/sm64_headless.us.wasm.zip.xor $(WASM)/sm64_headless.us.wasm.zip $(ROM_FILE); \
		python3 maketools/zip.py $(WASM)/sm64.us.wasm $(WASM)/sm64.us.wasm.zip; \
		python3 maketools/zip.py $(WASM)/sm64_headless.us.wasm $(WASM)/sm64_headless.us.wasm.zip; \
	else \
		echo "$(BLUE)\nNORMAL$(RESET)"; \
		$(MAKE) -C $(SM64PC) -j TARGET_WEB=1; \
		echo "$(BLUE)\nHEADLESS$(RESET)"; \
		$(MAKE) -C $(SM64PC) -j TARGET_WEB=1 HEADLESS_VERSION=1; \
		cp $(SM64PC)/build/us_web/sm64.us.js $(WASM_JS)/sm64.us.js; \
		cp $(SM64PC)/build/us_web/sm64.us.wasm $(WASM)/sm64.us.wasm; \
		cp $(SM64PC)/build/us_web_headless/sm64.us.wasm $(WASM)/sm64_headless.us.wasm; \
	fi

    # zip
	python3 maketools/zip.py $(WASM)/sm64.us.wasm $(WASM)/sm64.us.wasm.zip
	python3 maketools/zip.py $(WASM)/sm64_headless.us.wasm $(WASM)/sm64_headless.us.wasm.zip

    # xor
	python3 maketools/xor.py $(WASM)/sm64.us.wasm.zip $(WASM_ZIP_XOR)/sm64.us.wasm.zip.xor $(ROM_FILE);
	python3 maketools/xor.py $(WASM)/sm64_headless.us.wasm.zip $(WASM_ZIP_XOR)/sm64_headless.us.wasm.zip.xor $(ROM_FILE);


# 	cp $(WASM_JS)/sm64.us.js $(JS_PKG)/sm64.us.js
# 	cp $(WASM_ZIP_XOR)/sm64.us.wasm.xor $(JS_PKG)/sm64.us.wasm.xor;

# 	cp $(WASM_ZIP_XOR)/sm64_headless.us.wasm.xor $(JS_PKG)/sm64_headless.us.wasm.xor
# 	cp $(WASM_ZIP_XOR)/sm64_headless.us.wasm.xor $(PYTHON_PKG)/sm64_headless.us.wasm.xor

	cp $(WASM_ZIP_XOR)/sm64_headless.us.wasm.zip.xor $(RUST_PKG)/sm64_headless.us.wasm.zip.xor

	cp $(WASM_JS)/sm64.us.js $(JS_PKG)/sm64.us.js
	cp $(WASM_ZIP_XOR)/sm64.us.wasm.zip.xor $(JS_PKG)/sm64.us.wasm.zip.xor

# 	wasm-pack build browser-wasm --release -t bundler -d $(WEB_PROD)

# Define a target to copy the ROM file into the SM64PC
copy-rom:
	@echo "Copying $(ROM_FILE) to $(SM64PC)..."
	@if [ ! -f $(ROM_FILE) ]; then \
		echo -e "\033[0;31mA Mario 64 ROM is required with the name $(ROM_FILE)\033[0m"; \
		exit 1; \
	fi
	
	cp $(ROM_FILE) $(SM64PC)/$(ROM_FILE)

clean:
	@echo "Cleaning up..."
	$(MAKE) -C $(SM64PC) clean

.PHONY: all web-build cli-build copy-rom clean

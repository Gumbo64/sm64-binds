# maketools

Packaging helpers used by the root `Makefile` to build the distributable WASM
payloads without shipping the ROM itself.

The distributed artifact is a WASM module zipped and then XOR'd against a US
`.z64` ROM (`WASM_ZIP_XOR/*.xor`). Combining it back with the ROM at runtime
reconstructs the module, which keeps the copyrighted bytes out of the repo.

| Script | Purpose |
| --- | --- |
| `zip.py` | Compresses a built WASM module into the single-entry ZIP the bindings expect. |
| `unzip.py` | Extracts that ZIP back to a `.wasm` file. |
| `xor.py` | XORs a file with a key file, producing (or reversing) the `.xor` payload. |
| `tools.py` | The shared implementations plus their unit tests. |

## Tests

```
pytest maketools
```

The tests that recombine the shipped payload skip when no `baserom.us.z64` is
present at the repo root.

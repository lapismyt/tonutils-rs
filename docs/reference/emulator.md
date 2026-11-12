# Emulator (`tonutils::emulator`)

`tonutils::emulator` is an opt-in, runtime-loaded wrapper around
`ton-blockchain/ton`'s `libemulator`. It deliberately does not build C++ code
and does not vendor a prebuilt binary. The `emulator` feature adds `tvm`, `tlb`,
`libloading`, and `serde_json`, but no native build dependency.

## ABI ownership

`crates/tonutils/src/emulator/ffi.rs` mirrors
`emulator/emulator-extern.h`. Every used symbol is resolved once when an
`EmulatorEngine` is loaded:

- transaction emulator create/destroy, emulate, tick/tock, and all setters;
- TVM emulator create/destroy, get-method, external/internal send, and all
  setters;
- `emulator_set_verbosity_level`, `emulator_version`, `string_destroy`.

Safe wrappers keep an `Arc<EmulatorEngine>` for the lifetime of every emulator
object. Returned `const char *` values are copied with `CStr` and then released
with `string_destroy`; they are never freed through Rust's allocator. Emulator
handles are opaque `void *` values and are not sent across threads.

Skipped initially: the optimized `tvm_emulator_emulate_run_method*` request BoC
API and the config-object APIs (`emulator_config_create` and
`*_set_config_object`). All equivalent configuration knobs are available
through typed setters, including `C7Config.config`.

## Library resolution

Resolution order:

1. `EmulatorEngine::open(path)` for an explicit path;
2. `TONUTILS_EMULATOR_LIB`;
3. `TON_EMULATOR_LIB`;
4. the library downloaded by the `emulator-download` feature, if that
   build-time path still exists;
5. `libemulator.<platform extension>` on the dynamic-loader search path.

The library is loaded with `libloading::Library`. A missing symbol fails the
engine load with `EmulatorError::MissingSymbol` instead of panicking or
silently degrading.

## Compile-time download

The `emulator-download` feature enables `build.rs`. It maps the Cargo target to
an upstream release asset and downloads it into `OUT_DIR`:

| Target | Asset |
| --- | --- |
| Linux x86_64 | `libemulator-linux-x86_64.so` |
| Linux aarch64 | `libemulator-linux-arm64.so` |
| macOS x86_64 | `libemulator-mac-x86-64.dylib` |
| macOS aarch64 | `libemulator-mac-arm64.dylib` |
| Windows x86_64 | `libemulator.dll` |

By default the latest GitHub release is used. Set
`TONUTILS_EMULATOR_RELEASE=v2026.10` to pin a tag, or
`TONUTILS_EMULATOR_LIB_URL=https://...` to use an internal mirror. The build
requires `curl`. The download is skipped on docs.rs (`DOCS_RS` is set) and the
result is cached inside the Cargo `OUT_DIR` for the selected release.

The feature does **not** statically link `libemulator`. GitHub releases publish
only shared libraries, and the `ton-*` archives contain `libemulator.so`, not an
`libemulator.a`. Static linking would require building the upstream C++ tree
with `-DEMULATOR_STATIC=ON`, so this crate intentionally leaves that to
downstream packaging.

## Configuration root

`libemulator` accepts the raw `Hashmap 32 ^Cell` config dictionary root. It does
**not** accept the outer `ConfigParams` cell. `BlockchainConfig` therefore:

- `from_dictionary(cell)` wraps a dictionary root;
- `from_config_params(&ConfigParams)` takes `.config`;
- `from_dictionary_boc(bytes)` decodes a dictionary-root BoC;
- `from_config_params_boc(bytes)` decodes `ConfigParams` and extracts `.config`;
- `from_file`/`from_bytes` accept raw BoC bytes or base64 text.

The crate does not embed a default mainnet config. Emulation determinism and
trust depend on the caller supplying the intended config snapshot.

## Standard stack codec

`tvm::VmStack` implements the upstream `crypto/vm/stack.cpp` format. It is
separate from `tvm::TvmStack`, which preserves provider-specific payloads using
a crate-local tag format.

Wire tags:

| Tag | Value |
| --- | --- |
| `00` | null |
| `01` + 64 bits | tiny signed integer |
| `02ff` | NaN |
| `0201_` + 257 bits | signed integer |
| `03` + `^Cell` | cell |
| `04` + ref + 10/10/3/3-bit ranges | slice |
| `05` + `^Cell` | builder snapshot |
| `07` + 16-bit length + tuple refs | tuple |

`06` continuations are rejected with a typed stack error. A decoded builder is
represented by its finalized cell; serializing it reproduces the same bits and
references.

Stack layout is bottom-to-top. The root stores a 24-bit depth, a reference to
the cons-list rest, and the topmost value inline. Each cons cell stores the
previous rest reference followed by one value. Tuples use the unusual upstream
root/pair layout, where the root stores `[head, last_value]` for length >= 2 and
pair cells store `[previous_head, value]`.

`VmStackValue::to_cell`/`from_boc` implement the single-value form expected by
the previous-blocks-info setters.

## BoC checksum

TON BoCs use CRC32C (Castagnoli), not CRC32/ISO-HDLC. `tvm::boc` now creates and
validates CRC32C trailers. This is required for every BoC returned by
`libemulator`, which serializes with `BagOfCells::Mode::WithCRC32C`. The crate
still exposes the generic `crate::crc::CRC32` constant for unrelated callers.

Input BoCs are serialized without a checksum; `libemulator` accepts either form.

## Response shapes

Transaction success:

```json
{
  "success": true,
  "transaction": "base64 BoC",
  "shard_account": "base64 BoC",
  "vm_log": "...",
  "actions": "base64 BoC or null",
  "elapsed_time": 0.02
}
```

Transaction failure:

```json
{
  "success": false,
  "error": "...",
  "external_not_accepted": false,
  "vm_exit_code": 0,
  "vm_log": "...",
  "elapsed_time": 0.02
}
```

`external_not_accepted` and the optional VM fields are preserved in
`EmulatorError::Emulation`.

Get-method success:

```json
{
  "success": true,
  "stack": "base64 standard VmStack BoC",
  "gas_used": "1212",
  "vm_exit_code": 0,
  "vm_log": "...",
  "missing_library": null
}
```

Message send success:

```json
{
  "success": true,
  "new_code": "base64 BoC",
  "new_data": "base64 BoC",
  "accepted": true,
  "vm_exit_code": 0,
  "vm_log": "...",
  "gas_used": "1212",
  "actions": "base64 BoC or null",
  "missing_library": null
}
```

`gas_used` is emitted as a JSON string by current upstream code; the parser also
accepts a number. Unknown response fields are discarded by typed parsing; use
`*_raw` methods to retain the complete JSON object for forward compatibility.

## Determinism and trust

Emulation is local and synchronous. It performs no network access. Results are
deterministic only for a fixed library build, config, shard account, message,
`unixtime`, `lt`, and random seed. Callers are responsible for all inputs and
for choosing a trusted `libemulator` binary.

In async applications, run emulation inside `tokio::task::spawn_blocking`.

## Verification

Offline:

```bash
cargo test -p tonutils --no-default-features --features emulator
RUSTDOCFLAGS="-D warnings" cargo doc -p tonutils --all-features --no-deps
```

Optional integration coverage requires a locally built or downloaded
`libemulator` and is not part of the default test matrix.

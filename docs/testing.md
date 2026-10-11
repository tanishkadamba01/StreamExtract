# StreamExtract Testing Guide

## Test Suite Structure

The StreamExtract testing matrix includes unit tests, integration tests, and security tests:

| Test Group | File | Description | Status |
|------------|------|-------------|--------|
| **Path Safety** | `streamextract-core/src/engine/safety.rs` | Path traversal (`..`), UNC paths, absolute paths, illegal characters, DOS reserved names, trailing dots/spaces | ✅ 12 tests passing |
| **Output Writer** | `streamextract-core/src/engine/output_writer.rs` | Directory creation, overwrite policies (`Skip`, `Overwrite`, `Rename`, `Abort`) | ✅ 5 tests passing |
| **Integrity** | `streamextract-core/src/engine/integrity.rs` | Incremental CRC-32 hashing, corrupted data detection, missing checksum handling | ✅ 3 tests passing |
| **Cancellation** | `streamextract-core/src/engine/cancellation.rs` | Thread-safe atomic cancellation tokens | ✅ 3 tests passing |
| **Space Estimator** | `streamextract-core/src/engine/space_estimator.rs` | Available space querying, safety margin calculations, overflow guards | ✅ 4 tests passing |
| **End-to-End Extraction** | `streamextract-core/tests/extraction_integration_test.rs` | Real ZIP and TAR archive generation, inspection, streaming extraction, checksum verification | ✅ 6 tests passing |

---

## Running Tests

### Run all tests in the workspace
```bash
cargo test --workspace
```

### Run specific integration tests
```bash
cargo test --test extraction_integration_test
```

### Run with full backtrace
```bash
RUST_BACKTRACE=1 cargo test --workspace
```

---

## Manual Verification with CLI

### 1. Build the CLI binary
```bash
cargo build --bin streamextract
```

### 2. Inspect an archive
Using `cargo run` (recommended across all platforms):
```bash
cargo run --bin streamextract -- inspect <path-to-archive.zip>
```
Or directly running the compiled binary:
```bash
# Windows PowerShell:
.\target\x86_64-pc-windows-gnu\debug\streamextract.exe inspect <path-to-archive.zip>
# Unix / default layout:
./target/debug/streamextract inspect <path-to-archive.zip>
```

### 3. Extract to a destination
Using `cargo run`:
```bash
cargo run --bin streamextract -- extract <path-to-archive.zip> <path-to-dest> --overwrite skip
```
Or directly running the compiled binary:
```bash
# Windows PowerShell:
.\target\x86_64-pc-windows-gnu\debug\streamextract.exe extract <path-to-archive.zip> <path-to-dest> --overwrite skip
# Unix / default layout:
./target/debug/streamextract extract <path-to-archive.zip> <path-to-dest> --overwrite skip
```

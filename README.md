# StreamExtract

A production-grade Windows streaming archive extraction application optimized for **extremely low disk overhead**, bounded memory consumption, and enterprise-grade security.

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

---

## Highlights

- 🚀 **Streaming & Bounded Memory**: Never buffers entire archives or giant files into RAM. Decompresses in configurable chunks (default: 1 MiB) directly to destination writers.
- 🛡️ **Hardened Path Sanitization**: Rigorously neutralizes path traversals (`..`), absolute prefixes (`C:\`, `/`), UNC shares (`\\`), Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`), trailing periods/spaces, and Alternate Data Streams (`:stream`).
- ⚡ **Incremental CRC-32 Verification**: Computes entry checksums on the fly during chunk writes with zero secondary read passes over disk.
- 🛑 **Archive Bomb Defenses**: Enforces configurable maximum expansion ratios (default 200:1) and entry limits to protect system resources.
- 📊 **Real-time Event Streaming**: Decoupled MPSC channel event pipeline for UI/CLI integration (`SessionStarted`, `EntryStarted`, `EntryChunkWritten`, `EntryCompleted`, `SessionCompleted`, etc.).
- 💾 **Progressive Storage Reclamation Foundation**: Comprehensive capability gating, recovery journaling, and NTFS sparse file analysis for reclaiming source disk blocks without data loss.

---

## Workspace Architecture

```
NEXTGENEXTRACTION/
├── Cargo.toml                  # Workspace root
├── .cargo/config.toml          # Toolchain and linker configuration
├── streamextract-core/         # Core library crate (UI-agnostic)
│   ├── src/
│   │   ├── engine/             # Streaming orchestrator, writer, integrity, space estimator, journal
│   │   ├── formats/            # ZIP and TAR/GZ adapters, capability analysis
│   │   ├── models/             # Typed entries, events, options
│   │   ├── platform/           # Windows filesystem APIs (NTFS detection, disk space)
│   │   └── errors.rs           # Rich error taxonomy
│   └── tests/
│       └── extraction_integration_test.rs # End-to-end integration test suite
├── streamextract-cli/          # High-performance CLI harness
│   └── src/
│       └── main.rs             # CLI subcommands: inspect, extract
└── docs/                       # Technical specifications & architecture
    ├── architecture.md
    ├── format-support.md
    ├── storage-reclamation.md
    ├── security.md
    └── testing.md
```

---

## Building and Running

### Prerequisites
- Rust 1.80+ (stable toolchain)
- A C/C++ linker (provided by MSVC Build Tools, MinGW-w64, or the self-contained workspace `tools/w64devkit`)

### Build
```bash
cargo build --workspace
```

### Run Tests
```bash
cargo test --workspace
```
Currently 32 unit and integration tests passing covering path safety, CRC-32 integrity, overwrite policies, cancellation tokens, space estimation, and archive extraction.

### CLI Usage

#### Inspect an Archive
Inspects archive contents, formats, compression methods, sizes, and reclamation eligibility without extracting anything to disk:
```bash
target/debug/streamextract inspect <ARCHIVE_PATH>
```

#### Extract an Archive
Streams files safely into the target directory with real-time progress and checksum verification:
```bash
target/debug/streamextract extract <ARCHIVE_PATH> <DESTINATION_PATH> [OPTIONS]
```

**Options:**
- `-b, --buffer-size-kib <KIB>`: Streaming chunk buffer size (64 to 32768 KiB, default: 1024)
- `-o, --overwrite <skip|overwrite|rename|abort>`: Policy for conflicting destination files (default: `skip`)
- `-r, --max-expansion-ratio <RATIO>`: Maximum uncompressed/compressed ratio limit (default: 200.0)
- `-v, --verbose`: Enable debug / trace logging

---

## Security Guarantees

1. **Path Safety**: Every entry path is sanitized before creation. Any path containing `..`, absolute drive letters, UNC prefixes, or reserved DOS names is rejected immediately.
2. **Atomic Writes & Overwrite Controls**: Files are created with explicit collision policies (`Skip`, `Overwrite`, `Rename`, `Abort`).
3. **No Unsafe Execution**: Reparse points, symlinks, and junction creations require explicit security flags and are disabled by default.
4. **Crash-Safe Journaling**: Recovery journals are written atomically (`.tmp` write followed by atomic rename) to prevent corruption during unexpected shutdowns.

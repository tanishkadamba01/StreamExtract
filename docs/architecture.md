# StreamExtract Architecture Specification

## 1. System Overview

StreamExtract is architected specifically to solve the low-disk-space problem common when extracting massive archives on Windows workstations. Traditional archive utilities decompress entire archives into temporary directories (requiring 2x-3x total archive size in free disk space) or load large segments into system memory. StreamExtract decompresses streamingly using bounded buffers directly to destination files, verifying checksums incrementally and reporting structured events.

```
┌────────────────────────────────────────────────────────┐
│                   Consumer Layer                       │
│        (CLI: streamextract-cli / GUI: Tauri)           │
└──────────────────────────┬─────────────────────────────┘
                           │ ExtractionOptions, Channel (tx, rx)
                           ▼
┌────────────────────────────────────────────────────────┐
│                   streamextract-core                   │
│                                                        │
│  ┌────────────────────┐      ┌──────────────────────┐  │
│  │   Archive Reader   │◄────►│   Format Adapters    │  │
│  │  & Inspection API  │      │     (ZIP, TAR)       │  │
│  └─────────┬──────────┘      └──────────────────────┘  │
│            │                                           │
│            ▼                                           │
│  ┌──────────────────────────────────────────────────┐  │
│  │             Streaming Orchestrator               │  │
│  │                                                  │  │
│  │   • Space Estimator   • Path Safety Guard        │  │
│  │   • Cancellation      • Expansion Ratio Watchdog │  │
│  └─────────┬───────────────────────────┬────────────┘  │
│            │                           │               │
│            ▼                           ▼               │
│  ┌──────────────────┐        ┌──────────────────────┐  │
│  │  Output Writer   │        │ Incremental Verifier │  │
│  │ (Safe flags,     │───────►│ (Chunk CRC-32,       │  │
│  │  Overwrite rules)│        │  Pass/Fail status)   │  │
│  └─────────┬────────┘        └──────────────────────┘  │
│            │                                           │
│            ▼                                           │
│  ┌──────────────────────────────────────────────────┐  │
│  │         Windows Platform Subsystem (Win32)       │  │
│  │                                                  │  │
│  │   • GetVolumeInformationW (NTFS detection)       │  │
│  │   • GetDiskFreeSpaceExW (Accurate volume quotas) │  │
│  │   • FSCTL_SET_SPARSE / FSCTL_SET_ZERO_DATA       │  │
│  └──────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘
```

## 2. Core Modules

### 2.1 `engine::streaming`
The primary execution loop. It receives an `ExtractionOptions` configuration, a `CancellationToken`, and a typed `Sender<ExtractionEvent>`:
1. Validates options (buffer sizes within 64 KiB – 32 MiB, destination validity).
2. Performs read-only inspection of the archive to obtain metadata and capability reports.
3. Queries source and destination volume free space via Windows APIs.
4. Streams entries sequentially without holding large buffers in RAM.
5. Emits typed events at each state transition.

### 2.2 `engine::output_writer`
Handles destination file and directory operations with strict safety controls:
- **Directory Traversal Protection**: Ensures every directory in the target path is checked against reparse points and symlink injection.
- **Overwrite Policies**:
  - `Skip`: Leaves existing files unchanged.
  - `Overwrite`: Replaces existing files safely.
  - `Rename`: Generates collision-free names (e.g., `file (1).txt`).
  - `Abort`: Halts execution immediately upon encountering an existing target.

### 2.3 `engine::integrity`
Maintains an incremental CRC-32 hash (`crc32fast::Hasher`) updated chunk-by-chunk as data is written. When the last chunk of an entry is flushed, the computed CRC-32 is checked against the header's expected checksum. This eliminates the need for a secondary read pass.

### 2.4 `engine::safety`
Enforces path canonicalization and Windows-specific restrictions:
- Rejects absolute paths (`C:\...`, `/...`).
- Rejects UNC paths (`\\server\share`).
- Rejects parent directory traversal (`..`).
- Rejects Windows illegal characters (`<`, `>`, `:`, `"`, `|`, `?`, `*`).
- Rejects DOS reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`).
- Rejects trailing spaces and dots which Windows Win32 APIs automatically strip, preventing collision hijacking.

### 2.5 `engine::recovery_journal`
Provides a crash-resilient journal format (`.streamextract_journal`). State mutations are written to temporary files and atomically renamed, guaranteeing consistency even during sudden power failure or crashes.

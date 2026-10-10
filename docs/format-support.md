# Format Support & Reclamation Matrix

## Supported Formats

| Format | Magic Bytes / Header | Decompression | Reclamation Capability | Notes |
|--------|----------------------|---------------|------------------------|-------|
| **ZIP** (Deflate) | `PK\x03\x04` / `PK\x05\x06` | Native pure-Rust (`flate2`/`miniz_oxide`) | ⚠️ Phase 6 Investigation (Sparse range punching) | Independent entries; Central Directory at EOF |
| **ZIP** (Stored) | `PK\x03\x04` | Uncompressed | ⚠️ Phase 6 Investigation | No decompression overhead |
| **ZIP** (Bzip2) | `PK\x03\x04` | Native `bzip2` | ⚠️ Phase 6 Investigation | Independent entries |
| **ZIP** (Zstandard) | `PK\x03\x04` | Native `zstd` | ⚠️ Phase 6 Investigation | Independent entries |
| **TAR** (Bare) | `ustar\0` / GNU header | Sequential Stream | ❌ Not Supported (Solid stream) | Entries stored back-to-back without random access |
| **TAR.GZ** (Gzip) | `\x1f\x8b` | Streaming `flate2` | ❌ Not Supported | Wrapped solid stream; punching ruins compressed framing |
| **TAR.BZ2** (Bzip2) | `BZh` | Streaming `bzip2` | ❌ Not Supported | Wrapped solid stream |
| **TAR.XZ** (XZ/LZMA) | `\xfd7zXZ\0` | Streaming `xz2` | ❌ Not Supported | Wrapped solid stream |
| **TAR.ZST** (Zstandard) | `\x28\xb5\x2f\xfd` | Streaming `zstd` | ❌ Not Supported | Wrapped solid stream |

---

## Solid vs Non-Solid Archives

An essential distinction for progressive space reclamation is whether entries are **solid** or **independent**:

1. **Non-Solid (e.g., standard ZIP)**:
   - Each entry is compressed independently into its own local file record.
   - The Central Directory at the end of the archive indexes exact file offsets.
   - Decompressing entry `N` does not require state from entries `1..N-1`.
   - *Reclamation potential*: If entry `N` is extracted and verified, its byte range `[offset..offset+len]` can theoretically be zeroed/punched on disk if the filesystem supports sparse files.

2. **Solid (e.g., TAR.GZ, TAR.XZ, 7z solid)**:
   - All files are concatenated into a single byte stream before compression.
   - Decompressing entry `N` requires reading and maintaining decoder state across all preceding entries.
   - Punching out early portions corrupts the decompression stream for subsequent entries.
   - *Reclamation potential*: Impossible without complex chunk-repacking or until the entire stream has finished decompressing.

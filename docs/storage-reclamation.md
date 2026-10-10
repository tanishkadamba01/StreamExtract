# Progressive Source-Storage Reclamation Analysis

## The Core Problem

When users operate on systems with extremely low free disk space (e.g., 20 GiB free space, a 15 GiB archive, and 35 GiB uncompressed contents), extraction typically fails with `Disk full`. Traditional extractors require that the archive file remains intact throughout the entire extraction process while the output directory expands, leading to disk exhaustion.

The concept of **progressive source-storage reclamation** is: as each file is extracted and cryptographically verified, the corresponding physical storage occupied by that entry in the source archive is released back to the filesystem.

---

## Technical Feasibility on Windows (NTFS)

### 1. NTFS Sparse Files & Range Deallocation
NTFS supports sparse files through two primary Win32 `DeviceIoControl` control codes:
- **`FSCTL_SET_SPARSE`**: Marks a file as sparse, allowing NTFS to allocate disk blocks on-demand.
- **`FSCTL_SET_ZERO_DATA`**: Given a byte range `[FileOffset, BeyondFinalZero]`, instructs NTFS to deallocate the physical clusters underlying that range without changing the apparent length or external layout of the file.

When a range is zeroed with `FSCTL_SET_ZERO_DATA`, subsequent reads of that range return zeroes without consuming physical disk space.

### 2. Constraints & Gotchas of NTFS Sparse Punching
- **Cluster Alignment**: NTFS only deallocates storage if the zeroed range covers one or more full filesystem clusters (typically 4096 bytes or 64 KiB on large volumes). Ranges smaller than the cluster size will not free any disk space.
- **Filesystem Limitations**:
  - Supported: **NTFS** (and ReFS with different behavioral characteristics).
  - Unsupported: **FAT32**, **exFAT** (USB flash drives commonly formatted with exFAT cannot punch sparse ranges).
- **Archive File Structure Integrity**:
  - In ZIP archives, the **Central Directory** resides at the very end of the file. Punching ranges from the middle of the archive leaves the Central Directory intact.
  - However, local file headers (`PK\x03\x04`) cannot be punched if any tool relies on scanning local headers sequentially.
  - If extraction is interrupted midway, standard archive utilities (WinRAR, 7-Zip, Windows Explorer) will encounter zeroes in the punched entries and report the archive as corrupted.
- **Antivirus & File Locking**: Windows Defender or third-party AV tools frequently lock archive files during read access, potentially causing `ERROR_SHARING_VIOLATION` or `ERROR_ACCESS_DENIED` during `DeviceIoControl` calls.

---

## StreamExtract Safety Gates

To prevent data loss, StreamExtract implements a strict capability gate before allowing destructive operations:

1. **Explicit Opt-in**: Destructive reclamation is disabled by default (`reclamation.enabled = false`) and requires explicit confirmation.
2. **Capability Check**: The archive format must report `ReclamationEligibility::Supported`. In Phase 1, all formats report `ReclamationEligibility::Unsupported` until the Phase 6 test protocol completes.
3. **Recovery Journaling**: Every byte range to be reclaimed must be logged in `.streamextract_journal` *prior* to committing the deallocation.
4. **Volume Check**: Destination and source must reside on NTFS volumes verified via `is_ntfs()`.
5. **Disposable File Verification**: Destructive tests must only be performed on cloned/disposable archive files with verified checksums.

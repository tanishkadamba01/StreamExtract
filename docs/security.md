# StreamExtract Threat Model & Security Controls

## 1. Threat Landscape

Extracting untrusted archives presents severe security risks on Windows systems. Attackers can craft malicious archives designed to achieve arbitrary code execution, file overwrite, denial of service, or directory traversal.

---

## 2. Security Vectors & Defenses

### 2.1 Directory Traversal (`Zip Slip`)
- **Vector**: Archive entries containing `../`, `..\`, or encoded path sequences designed to escape the output directory and overwrite arbitrary system files (e.g., `../../Windows/System32/evil.dll` or `..\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\run.bat`).
- **Defense**: StreamExtract parses all path components via `engine::safety::sanitise_path`. Any presence of `Component::ParentDir` (`..`) causes instant rejection with `ExtractionError::PathTraversal`. The canonical resolved path is verified to strictly begin with the destination directory.

### 2.2 Absolute & UNC Path Injection
- **Vector**: Entries beginning with absolute drive roots (e.g. `C:\file.txt`, `/etc/passwd`) or UNC network paths (`\\evil-server\share\file.txt`).
- **Defense**: Rejection of `Component::RootDir`, `Component::Prefix`, and paths starting with `\\` or `//`.

### 2.3 Windows Reserved Device Names
- **Vector**: Windows reserves legacy DOS device names across all directories. Creating a file named `CON`, `PRN`, `AUX`, `NUL`, `COM1`..`COM9`, or `LPT1`..`LPT9` (or `CON.txt`) can cause file lockups, BSODs, or application crashes.
- **Defense**: StreamExtract extracts the stem of every component and compares it case-insensitively against the complete set of Windows reserved names, rejecting matches with `ExtractionError::ReservedName`.

### 2.4 Trailing Spaces and Periods
- **Vector**: On Windows Win32 APIs, trailing periods (`.`) and spaces (` `) are silently stripped by the filesystem. An attacker could include `foo.txt ` to collide with or shadow an existing `foo.txt`.
- **Defense**: `sanitise_path` rejects any component ending in `.` or ` ` with `ExtractionError::IllegalFilename`.

### 2.5 Windows Alternate Data Streams (ADS)
- **Vector**: Colon characters (`:`) allow writing hidden streams behind existing files (e.g. `file.txt:hidden.exe`).
- **Defense**: All illegal Windows characters (`<`, `>`, `:`, `"`, `|`, `?`, `*`) and ASCII control characters (`0x00..0x1F`) are rejected.

### 2.6 Symlink & Reparse Point Hijacking
- **Vector**: An archive creates a directory symlink pointing to `C:\Windows`, and subsequent entries write into that directory, exploiting symlink resolution to overwrite system files.
- **Defense**: StreamExtract disables symlink creation and reparse point traversal by default (`permissions.allow_symlinks = false`, `permissions.allow_reparse_follow = false`). The output directory writer verifies path components to ensure no directory created is an existing symlink or reparse point.

### 2.7 Archive Bombs (Decompression Denial of Service)
- **Vector**: Highly compressed files (e.g. 42.zip: a few kilobytes expanding to petabytes of zeroes) exhausting all disk space and memory.
- **Defense**:
  - `max_expansion_ratio`: Monitored per-entry. If `uncompressed_bytes / compressed_bytes > max_ratio` (default: 200.0), extraction aborts.
  - `max_output_bytes`: Enforces global size ceilings.
  - `max_entry_count`: Limits total number of files.
  - Bounded buffers: Decompression never allocates memory proportional to entry size.

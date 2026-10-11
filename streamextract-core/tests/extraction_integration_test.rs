use std::fs::{self, File};
use std::io::Write;
use std::sync::mpsc::channel;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use streamextract_core::{
    engine::cancellation::CancellationToken,
    inspect_archive,
    models::{
        archive_entry::EntryKind,
        extraction_event::ExtractionEvent,
        extraction_options::{ExtractionOptions, OverwritePolicy},
    },
    run_extraction,
};

fn create_sample_zip(path: &std::path::Path) {
    let file = File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);

    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 1. A text file
    zip.start_file("hello.txt", options).unwrap();
    zip.write_all(b"Hello from StreamExtract test!").unwrap();

    // 2. A subdirectory file
    zip.add_directory("subfolder", options).unwrap();
    zip.start_file("subfolder/data.bin", options).unwrap();
    let sample_data = vec![0xAB; 1024];
    zip.write_all(&sample_data).unwrap();

    zip.finish().unwrap();
}

fn test_dir() -> TempDir {
    if let Some(cargo_tmp) = option_env!("CARGO_TARGET_TMPDIR") {
        let path = std::path::Path::new(cargo_tmp);
        let _ = fs::create_dir_all(path);
        if let Ok(td) = tempfile::tempdir_in(path) {
            return td;
        }
    }

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(ws_root) = manifest_dir.parent() {
        let ws_target_tmp = ws_root.join("target").join("tmp");
        let _ = fs::create_dir_all(&ws_target_tmp);
        if let Ok(td) = tempfile::tempdir_in(&ws_target_tmp) {
            return td;
        }
    }

    if let Ok(td) = TempDir::new() {
        return td;
    }

    let crate_target_tmp = manifest_dir.join("target").join("tmp");
    let _ = fs::create_dir_all(&crate_target_tmp);
    tempfile::tempdir_in(&crate_target_tmp)
        .expect("failed to create temporary test directory in any candidate location")
}

#[test]
fn test_zip_inspection() {
    let tmp = test_dir();
    let zip_path = tmp.path().join("test.zip");
    create_sample_zip(&zip_path);

    let options = ExtractionOptions::new(&zip_path, tmp.path().join("out"));
    let inspection = inspect_archive(&zip_path, &options).unwrap();

    assert_eq!(inspection.entries.len(), 3);
    assert_eq!(inspection.entries[0].raw_path, "hello.txt");
    assert_eq!(inspection.entries[0].kind, EntryKind::File);
    assert!(inspection.entries[0].uncompressed_size.unwrap() > 0);
    assert!(inspection.entries[0].integrity.crc32.is_some());

    assert_eq!(inspection.entries[1].raw_path, "subfolder/");
    assert_eq!(inspection.entries[1].kind, EntryKind::Directory);

    assert_eq!(inspection.entries[2].raw_path, "subfolder/data.bin");
    assert_eq!(inspection.entries[2].kind, EntryKind::File);
    assert_eq!(inspection.entries[2].uncompressed_size.unwrap(), 1024);
}

#[test]
fn test_zip_streaming_extraction_success() {
    let tmp = test_dir();
    let zip_path = tmp.path().join("test.zip");
    let dest_dir = tmp.path().join("extracted");
    create_sample_zip(&zip_path);

    let options = ExtractionOptions::new(&zip_path, &dest_dir);
    let cancel = CancellationToken::new();
    let (tx, rx) = channel();

    run_extraction(options, cancel, tx).unwrap();

    let mut events = Vec::new();
    while let Ok(event) = rx.recv() {
        events.push(event);
    }

    // Verify session events
    assert!(events.iter().any(|e| matches!(e, ExtractionEvent::SessionStarted { .. })));
    assert!(events.iter().any(|e| matches!(e, ExtractionEvent::SessionCompleted { .. })));

    // Verify extracted files exist on disk with correct content
    let extracted_hello = dest_dir.join("hello.txt");
    assert!(extracted_hello.exists());
    let hello_content = fs::read_to_string(&extracted_hello).unwrap();
    assert_eq!(hello_content, "Hello from StreamExtract test!");

    let extracted_bin = dest_dir.join("subfolder").join("data.bin");
    assert!(extracted_bin.exists());
    let bin_content = fs::read(&extracted_bin).unwrap();
    assert_eq!(bin_content.len(), 1024);
    assert_eq!(bin_content[0], 0xAB);
}

#[test]
fn test_zip_extraction_overwrite_policy_skip() {
    let tmp = test_dir();
    let zip_path = tmp.path().join("test.zip");
    let dest_dir = tmp.path().join("extracted");
    create_sample_zip(&zip_path);

    // Pre-create the file with custom content
    fs::create_dir_all(&dest_dir).unwrap();
    let target = dest_dir.join("hello.txt");
    fs::write(&target, b"original unmodified content").unwrap();

    let mut options = ExtractionOptions::new(&zip_path, &dest_dir);
    options.overwrite_policy = OverwritePolicy::Skip;
    let cancel = CancellationToken::new();
    let (tx, _rx) = channel();

    run_extraction(options, cancel, tx).unwrap();

    // Verify original content was preserved
    let content = fs::read_to_string(&target).unwrap();
    assert_eq!(content, "original unmodified content");
}

#[test]
fn test_zip_extraction_cancellation() {
    let tmp = test_dir();
    let zip_path = tmp.path().join("test.zip");
    let dest_dir = tmp.path().join("extracted");
    create_sample_zip(&zip_path);

    let options = ExtractionOptions::new(&zip_path, &dest_dir);
    let cancel = CancellationToken::new();
    // Cancel immediately before extraction starts
    cancel.cancel();

    let (tx, _rx) = channel();
    let result = run_extraction(options, cancel, tx);
    assert!(result.is_err());
}

#[test]
fn test_tar_inspection_and_extraction() {
    let tmp = test_dir();
    let tar_path = tmp.path().join("test.tar");
    let dest_dir = tmp.path().join("tar_out");

    // Create a bare TAR archive
    {
        let file = File::create(&tar_path).unwrap();
        let mut builder = tar::Builder::new(file);

        let data = b"Hello from TAR file inside StreamExtract!";
        let mut header = tar::Header::new_gnu();
        header.set_path("tar_file.txt").unwrap();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &data[..]).unwrap();
        builder.finish().unwrap();
    }

    // Inspect
    let options = ExtractionOptions::new(&tar_path, &dest_dir);
    let inspection = inspect_archive(&tar_path, &options).unwrap();
    assert_eq!(inspection.entries.len(), 1);
    assert_eq!(inspection.entries[0].raw_path, "tar_file.txt");

    // Extract
    let cancel = CancellationToken::new();
    let (tx, rx) = channel();
    run_extraction(options, cancel, tx).unwrap();

    let mut completed = false;
    while let Ok(event) = rx.recv() {
        if matches!(event, ExtractionEvent::SessionCompleted { .. }) {
            completed = true;
        }
    }
    assert!(completed);

    let extracted = dest_dir.join("tar_file.txt");
    assert!(extracted.exists());
    assert_eq!(fs::read_to_string(extracted).unwrap(), "Hello from TAR file inside StreamExtract!");
}

#[test]
fn test_security_path_traversal_rejection() {
    let tmp = test_dir();
    let zip_path = tmp.path().join("evil.zip");
    let dest_dir = tmp.path().join("safe_out");

    // Construct a zip file containing a malicious path traversal entry
    {
        let file = File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        zip.start_file("../../evil.txt", options).unwrap();
        zip.write_all(b"malicious payload").unwrap();
        zip.finish().unwrap();
    }

    let options = ExtractionOptions::new(&zip_path, &dest_dir);
    let cancel = CancellationToken::new();
    let (tx, rx) = channel();

    // Run extraction
    let _ = run_extraction(options, cancel, tx);

    let mut events = Vec::new();
    while let Ok(event) = rx.recv() {
        events.push(event);
    }

    // Must emit EntryFailed or Warning due to path traversal, and file MUST NOT exist outside dest_dir
    let outside_file = tmp.path().join("evil.txt");
    assert!(!outside_file.exists(), "Path traversal vulnerability: file was written outside destination!");
}

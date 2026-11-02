use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_native_vfs_read_write() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("test.txt", b"hello world").await.unwrap();
    let content = vfs.read_to_string("test.txt").await.unwrap();

    assert_eq!(content, "hello world");
}

#[tokio::test]
async fn test_native_vfs_append() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.append("log.txt", b"line1\n").await.unwrap();
    vfs.append("log.txt", b"line2\n").await.unwrap();

    let content = vfs.read_to_string("log.txt").await.unwrap();
    assert_eq!(content, "line1\nline2\n");
}

#[tokio::test]
async fn test_native_vfs_nested_dirs() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("a/b/c/file.txt", b"deep").await.unwrap();
    let content = vfs.read_to_string("a/b/c/file.txt").await.unwrap();

    assert_eq!(content, "deep");
}

#[tokio::test]
async fn test_native_vfs_list_dir() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    // Create files and directories
    vfs.write("file1.txt", b"content1").await.unwrap();
    vfs.write("file2.txt", b"content2").await.unwrap();
    vfs.write("subdir/nested.txt", b"nested").await.unwrap();

    // List root directory
    let entries = vfs.list_dir("").await.unwrap();

    // Should have 3 entries: subdir (dir), file1.txt, file2.txt
    assert_eq!(entries.len(), 3);

    // Directory should come first
    assert_eq!(entries[0].name, "subdir");
    assert!(entries[0].is_directory);

    // Files should be alphabetically sorted
    assert_eq!(entries[1].name, "file1.txt");
    assert!(!entries[1].is_directory);
    assert_eq!(entries[2].name, "file2.txt");
    assert!(!entries[2].is_directory);
}

// ─── NativeVfs: exists, remove, rename ───────────────────────────────

#[tokio::test]
async fn test_native_vfs_exists_returns_false_for_missing_file() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    assert!(!vfs.exists("nope.txt").await.unwrap());
}

#[tokio::test]
async fn test_native_vfs_exists_returns_true_after_write() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("yes.txt", b"data").await.unwrap();
    assert!(vfs.exists("yes.txt").await.unwrap());
}

#[tokio::test]
async fn test_native_vfs_remove_deletes_file() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("doomed.txt", b"bye").await.unwrap();
    assert!(vfs.exists("doomed.txt").await.unwrap());

    vfs.remove("doomed.txt").await.unwrap();
    assert!(!vfs.exists("doomed.txt").await.unwrap());
}

#[tokio::test]
async fn test_native_vfs_remove_nonexistent_returns_error() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    let result = vfs.remove("ghost.txt").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_native_vfs_rename_moves_file() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("old.txt", b"content").await.unwrap();
    vfs.rename("old.txt", "new.txt").await.unwrap();

    assert!(!vfs.exists("old.txt").await.unwrap());
    let content = vfs.read_to_string("new.txt").await.unwrap();
    assert_eq!(content, "content");
}

#[tokio::test]
async fn test_native_vfs_create_dir_all() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.create_dir_all("x/y/z").await.unwrap();
    vfs.write("x/y/z/deep.txt", b"found").await.unwrap();
    let content = vfs.read_to_string("x/y/z/deep.txt").await.unwrap();
    assert_eq!(content, "found");
}

// ─── NativeVfs: atomic write semantics ───────────────────────────────

#[tokio::test]
async fn test_native_vfs_write_overwrites_existing() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("over.txt", b"first").await.unwrap();
    vfs.write("over.txt", b"second").await.unwrap();
    let content = vfs.read_to_string("over.txt").await.unwrap();
    assert_eq!(content, "second");
}

#[tokio::test]
async fn test_native_vfs_write_empty_file() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("empty.txt", b"").await.unwrap();
    let content = vfs.read("empty.txt").await.unwrap();
    assert!(content.is_empty());
}

#[tokio::test]
async fn test_native_vfs_write_no_leftover_tmp() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("clean.txt", b"data").await.unwrap();

    // No .tmp file should remain after atomic write
    assert!(!vfs.exists("clean.tmp").await.unwrap());
}

// ─── NativeVfs: read errors ──────────────────────────────────────────

#[tokio::test]
async fn test_native_vfs_read_nonexistent_returns_not_found() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    let result = vfs.read("missing.txt").await;
    assert!(matches!(result, Err(VfsError::NotFound(_))));
}

#[tokio::test]
async fn test_native_vfs_read_to_string_nonexistent_returns_not_found() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    let result = vfs.read_to_string("missing.txt").await;
    assert!(matches!(result, Err(VfsError::NotFound(_))));
}

// ─── NativeVfs: append edge cases ────────────────────────────────────

#[tokio::test]
async fn test_native_vfs_append_creates_file_if_missing() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.append("new.txt", b"first").await.unwrap();
    let content = vfs.read_to_string("new.txt").await.unwrap();
    assert_eq!(content, "first");
}

#[tokio::test]
async fn test_native_vfs_append_multiple_sequential() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    for i in 0..10 {
        vfs.append("journal.log", format!("entry-{}\n", i).as_bytes())
            .await
            .unwrap();
    }

    let content = vfs.read_to_string("journal.log").await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 10);
    assert_eq!(lines[0], "entry-0");
    assert_eq!(lines[9], "entry-9");
}

// ─── NativeVfs: binary data ──────────────────────────────────────────

#[tokio::test]
async fn test_native_vfs_read_write_binary() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    let binary: Vec<u8> = (0u8..=255).collect();
    vfs.write("binary.bin", &binary).await.unwrap();
    let read_back = vfs.read("binary.bin").await.unwrap();
    assert_eq!(read_back, binary);
}

// ─── NativeVfs: path traversal prevention ────────────────────────────

#[tokio::test]
async fn test_native_vfs_resolve_strips_path_traversal() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    // Path traversal attempts should be stripped
    vfs.write("../../../etc/passwd", b"nope").await.unwrap();
    // File should be created inside the base dir, not at /etc/passwd
    assert!(vfs.exists("etc/passwd").await.unwrap());
}

#[tokio::test]
async fn test_native_vfs_resolve_strips_leading_slash() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("/absolute.txt", b"data").await.unwrap();
    assert!(vfs.exists("absolute.txt").await.unwrap());
}

// ─── NativeVfs: list_dir edge cases ──────────────────────────────────

#[tokio::test]
async fn test_native_vfs_list_dir_empty() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    let entries = vfs.list_dir("").await.unwrap();
    assert!(entries.is_empty());
}

#[tokio::test]
async fn test_native_vfs_list_dir_nonexistent_returns_error() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    let result = vfs.list_dir("nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_native_vfs_list_dir_case_insensitive_sort() {
    let temp = TempDir::new().unwrap();
    let vfs = NativeVfs::new(temp.path());

    vfs.write("Zebra.txt", b"z").await.unwrap();
    vfs.write("apple.txt", b"a").await.unwrap();
    vfs.write("Banana.txt", b"b").await.unwrap();

    let entries = vfs.list_dir("").await.unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].name, "apple.txt");
    assert_eq!(entries[1].name, "Banana.txt");
    assert_eq!(entries[2].name, "Zebra.txt");
}

// ─── get_platform_vfs() ──────────────────────────────────────────────

#[tokio::test]
async fn test_get_platform_vfs_returns_boxed_vfs() {
    // get_platform_vfs() returns Box<dyn Vfs + Send + Sync> on all platforms.
    // We can't control the root dir, but we can verify it returns a valid object.
    let vfs = get_platform_vfs();
    // Smoke test: exists should work even if path doesn't exist
    let result = vfs.exists("__nonexistent_test_file__").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_dyn_vfs_supports_all_operations() {
    let temp = TempDir::new().unwrap();
    let vfs: Box<dyn Vfs + Send + Sync> = Box::new(NativeVfs::new(temp.path()));

    // write + read
    vfs.write("test.txt", b"hello").await.unwrap();
    assert_eq!(vfs.read("test.txt").await.unwrap(), b"hello");

    // read_to_string
    assert_eq!(vfs.read_to_string("test.txt").await.unwrap(), "hello");

    // exists
    assert!(vfs.exists("test.txt").await.unwrap());
    assert!(!vfs.exists("nope.txt").await.unwrap());

    // append
    vfs.append("test.txt", b" world").await.unwrap();
    assert_eq!(vfs.read_to_string("test.txt").await.unwrap(), "hello world");

    // create_dir_all + nested write
    vfs.create_dir_all("a/b").await.unwrap();
    vfs.write("a/b/nested.txt", b"deep").await.unwrap();
    assert_eq!(vfs.read_to_string("a/b/nested.txt").await.unwrap(), "deep");

    // rename
    vfs.rename("test.txt", "moved.txt").await.unwrap();
    assert!(!vfs.exists("test.txt").await.unwrap());
    assert_eq!(
        vfs.read_to_string("moved.txt").await.unwrap(),
        "hello world"
    );

    // list_dir
    let entries = vfs.list_dir("").await.unwrap();
    assert!(entries.len() >= 2);

    // remove
    vfs.remove("moved.txt").await.unwrap();
    assert!(!vfs.exists("moved.txt").await.unwrap());
}

#[tokio::test]
async fn test_dyn_vfs_can_be_wrapped_in_arc() {
    let temp = TempDir::new().unwrap();
    let vfs: std::sync::Arc<dyn Vfs + Send + Sync> =
        std::sync::Arc::from(Box::new(NativeVfs::new(temp.path())) as Box<dyn Vfs + Send + Sync>);

    // Verify Arc<dyn Vfs> works for Distributed/Persistent mount patterns
    let vfs_clone = vfs.clone();
    vfs.write("arc_test.txt", b"shared").await.unwrap();
    let content = vfs_clone.read_to_string("arc_test.txt").await.unwrap();
    assert_eq!(content, "shared");
}

// ─── UringVfs tests (Linux only, gated behind feature) ──────────────

#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod uring_tests {
    use super::*;

    #[tokio::test]
    async fn test_uring_vfs_creation() {
        let temp = TempDir::new().unwrap();
        let result = super::super::UringVfs::new(temp.path());
        assert!(
            result.is_ok(),
            "UringVfs::new should succeed on supported Linux kernels"
        );
    }

    #[tokio::test]
    async fn test_uring_vfs_read_write() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("test.txt", b"hello world").await.unwrap();
        let content = vfs.read_to_string("test.txt").await.unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_uring_vfs_read_bytes() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        let data = b"binary data \x00\x01\x02\xff";
        vfs.write("bin.dat", data).await.unwrap();
        let read_back = vfs.read("bin.dat").await.unwrap();
        assert_eq!(read_back, data);
    }

    #[tokio::test]
    async fn test_uring_vfs_append() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.append("log.txt", b"line1\n").await.unwrap();
        vfs.append("log.txt", b"line2\n").await.unwrap();

        let content = vfs.read_to_string("log.txt").await.unwrap();
        assert_eq!(content, "line1\nline2\n");
    }

    #[tokio::test]
    async fn test_uring_vfs_append_creates_file() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.append("new.log", b"first entry\n").await.unwrap();
        let content = vfs.read_to_string("new.log").await.unwrap();
        assert_eq!(content, "first entry\n");
    }

    #[tokio::test]
    async fn test_uring_vfs_append_many_sequential() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        for i in 0..100 {
            vfs.append("journal.log", format!("entry-{}\n", i).as_bytes())
                .await
                .unwrap();
        }

        let content = vfs.read_to_string("journal.log").await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 100);
        assert_eq!(lines[0], "entry-0");
        assert_eq!(lines[99], "entry-99");
    }

    #[tokio::test]
    async fn test_uring_vfs_nested_dirs() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("a/b/c/file.txt", b"deep").await.unwrap();
        let content = vfs.read_to_string("a/b/c/file.txt").await.unwrap();
        assert_eq!(content, "deep");
    }

    #[tokio::test]
    async fn test_uring_vfs_exists() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        assert!(!vfs.exists("missing.txt").await.unwrap());
        vfs.write("present.txt", b"here").await.unwrap();
        assert!(vfs.exists("present.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_uring_vfs_remove() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("doomed.txt", b"bye").await.unwrap();
        assert!(vfs.exists("doomed.txt").await.unwrap());

        vfs.remove("doomed.txt").await.unwrap();
        assert!(!vfs.exists("doomed.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_uring_vfs_remove_nonexistent_returns_error() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        let result = vfs.remove("ghost.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_uring_vfs_rename() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("old.txt", b"content").await.unwrap();
        vfs.rename("old.txt", "new.txt").await.unwrap();

        assert!(!vfs.exists("old.txt").await.unwrap());
        let content = vfs.read_to_string("new.txt").await.unwrap();
        assert_eq!(content, "content");
    }

    #[tokio::test]
    async fn test_uring_vfs_create_dir_all() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.create_dir_all("x/y/z").await.unwrap();
        vfs.write("x/y/z/deep.txt", b"found").await.unwrap();
        let content = vfs.read_to_string("x/y/z/deep.txt").await.unwrap();
        assert_eq!(content, "found");
    }

    #[tokio::test]
    async fn test_uring_vfs_list_dir() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("file1.txt", b"1").await.unwrap();
        vfs.write("file2.txt", b"2").await.unwrap();
        vfs.write("subdir/nested.txt", b"n").await.unwrap();

        let entries = vfs.list_dir("").await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "subdir");
        assert!(entries[0].is_directory);
        assert_eq!(entries[1].name, "file1.txt");
        assert_eq!(entries[2].name, "file2.txt");
    }

    #[tokio::test]
    async fn test_uring_vfs_list_dir_empty() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        let entries = vfs.list_dir("").await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_uring_vfs_atomic_write_overwrites() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("over.txt", b"first").await.unwrap();
        vfs.write("over.txt", b"second").await.unwrap();
        let content = vfs.read_to_string("over.txt").await.unwrap();
        assert_eq!(content, "second");
    }

    #[tokio::test]
    async fn test_uring_vfs_atomic_write_no_leftover_tmp() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("clean.txt", b"data").await.unwrap();
        assert!(!vfs.exists("clean.tmp").await.unwrap());
    }

    #[tokio::test]
    async fn test_uring_vfs_write_empty_file() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("empty.txt", b"").await.unwrap();
        let content = vfs.read("empty.txt").await.unwrap();
        assert!(content.is_empty());
    }

    #[tokio::test]
    async fn test_uring_vfs_read_nonexistent_returns_error() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        let result = vfs.read("missing.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_uring_vfs_binary_roundtrip() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        let binary: Vec<u8> = (0u8..=255).collect();
        vfs.write("binary.bin", &binary).await.unwrap();
        let read_back = vfs.read("binary.bin").await.unwrap();
        assert_eq!(read_back, binary);
    }

    #[tokio::test]
    async fn test_uring_vfs_large_file_roundtrip() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        // 1 MiB file
        let large: Vec<u8> = (0..1_048_576).map(|i| (i % 256) as u8).collect();
        vfs.write("large.bin", &large).await.unwrap();
        let read_back = vfs.read("large.bin").await.unwrap();
        assert_eq!(read_back.len(), large.len());
        assert_eq!(read_back, large);
    }

    #[tokio::test]
    async fn test_uring_vfs_path_traversal_prevention() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        vfs.write("../../../etc/passwd", b"nope").await.unwrap();
        assert!(vfs.exists("etc/passwd").await.unwrap());
    }

    #[tokio::test]
    async fn test_uring_vfs_concurrent_reads() {
        let temp = TempDir::new().unwrap();
        let vfs = std::sync::Arc::new(super::super::UringVfs::new(temp.path()).unwrap());

        // Write several files
        for i in 0..10 {
            vfs.write(
                &format!("file_{}.txt", i),
                format!("content_{}", i).as_bytes(),
            )
            .await
            .unwrap();
        }

        // Read them all concurrently
        let mut handles = Vec::new();
        for i in 0..10 {
            let vfs = vfs.clone();
            handles.push(tokio::spawn(async move {
                let content = vfs
                    .read_to_string(&format!("file_{}.txt", i))
                    .await
                    .unwrap();
                assert_eq!(content, format!("content_{}", i));
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_uring_vfs_concurrent_appends() {
        let temp = TempDir::new().unwrap();
        let vfs = std::sync::Arc::new(super::super::UringVfs::new(temp.path()).unwrap());

        // 50 concurrent appends
        let mut handles = Vec::new();
        for i in 0..50 {
            let vfs = vfs.clone();
            handles.push(tokio::spawn(async move {
                vfs.append("concurrent.log", format!("entry-{}\n", i).as_bytes())
                    .await
                    .unwrap();
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let content = vfs.read_to_string("concurrent.log").await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 50);
    }

    #[tokio::test]
    async fn test_uring_vfs_worker_shutdown_clean() {
        let temp = TempDir::new().unwrap();

        // Create and immediately drop — should shut down cleanly
        {
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();
            vfs.write("shutdown_test.txt", b"data").await.unwrap();
        }
        // If we reach here without hanging, the worker shut down cleanly.

        // Verify the data persisted
        let vfs = NativeVfs::new(temp.path());
        let content = vfs.read_to_string("shutdown_test.txt").await.unwrap();
        assert_eq!(content, "data");
    }

    #[tokio::test]
    async fn test_uring_vfs_operations_after_many_writes() {
        let temp = TempDir::new().unwrap();
        let vfs = super::super::UringVfs::new(temp.path()).unwrap();

        // Stress test: many sequential writes then reads
        for i in 0..200 {
            let name = format!("file_{}.txt", i);
            let content = format!("content_{}", i);
            vfs.write(&name, content.as_bytes()).await.unwrap();
        }

        for i in 0..200 {
            let name = format!("file_{}.txt", i);
            let expected = format!("content_{}", i);
            let actual = vfs.read_to_string(&name).await.unwrap();
            assert_eq!(actual, expected, "file {} content mismatch", i);
        }
    }

    #[tokio::test]
    async fn test_uring_vfs_matches_native_vfs_behavior() {
        let temp1 = TempDir::new().unwrap();
        let temp2 = TempDir::new().unwrap();
        let uring = super::super::UringVfs::new(temp1.path()).unwrap();
        let native = NativeVfs::new(temp2.path());

        // Both should produce identical results for the same operations
        let ops = vec![
            ("write", "test.txt", b"hello" as &[u8]),
            ("write", "dir/nested.txt", b"nested"),
            ("append", "log.txt", b"line1\n"),
            ("append", "log.txt", b"line2\n"),
        ];

        for (op, path, data) in &ops {
            match *op {
                "write" => {
                    uring.write(path, data).await.unwrap();
                    native.write(path, data).await.unwrap();
                }
                "append" => {
                    uring.append(path, data).await.unwrap();
                    native.append(path, data).await.unwrap();
                }
                _ => unreachable!(),
            }
        }

        // Compare results
        for path in &["test.txt", "dir/nested.txt", "log.txt"] {
            let uring_content = uring.read(path).await.unwrap();
            let native_content = native.read(path).await.unwrap();
            assert_eq!(uring_content, native_content, "mismatch for {}", path);
        }

        // Compare list_dir
        let uring_entries = uring.list_dir("").await.unwrap();
        let native_entries = native.list_dir("").await.unwrap();
        assert_eq!(uring_entries.len(), native_entries.len());
        for (u, n) in uring_entries.iter().zip(native_entries.iter()) {
            assert_eq!(u.name, n.name);
            assert_eq!(u.is_directory, n.is_directory);
        }
    }
}

// ─── UringVfs fallback test (Linux without io_uring support) ─────────

#[cfg(all(target_os = "linux", feature = "io-uring"))]
#[tokio::test]
async fn test_uring_vfs_fallback_on_unsupported() {
    // get_platform_vfs should always return a working VFS,
    // regardless of whether io_uring is actually available.
    let temp = TempDir::new().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let vfs = get_platform_vfs();
    vfs.write("fallback.txt", b"works").await.unwrap();
    let content = vfs.read_to_string("fallback.txt").await.unwrap();
    assert_eq!(content, "works");
}

//! Phase 3c (FINISH_INTERPRETER.md) — VFS threaded into the interpreter.
//!
//! The browser Studio holds a `WebVfs`; the interpreter must route file I/O
//! (`Write`/`Read`/`Mount`) to whatever VFS it is handed. This proves the native
//! threading with a `NativeVfs` write→read roundtrip; the browser uses the exact
//! same `.with_vfs` seam with a `WebVfs`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use logicaffeine_compile::interpret_streaming_with_vfs;
use logicaffeine_system::fs::{NativeVfs, Vfs};

fn sink() -> (Rc<RefCell<Vec<String>>>, Rc<RefCell<impl FnMut(String)>>) {
    let lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let collected = lines.clone();
    let callback = Rc::new(RefCell::new(move |line: String| collected.borrow_mut().push(line)));
    (lines, callback)
}

#[tokio::test]
async fn interp_streaming_threads_vfs_write_read_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vfs: Arc<dyn Vfs> = Arc::new(NativeVfs::new(dir.path()));

    let src = "## Main\n\
        Write \"hello world\" to file \"out.txt\".\n\
        Read content from file \"out.txt\".\n\
        Show content.\n";

    let (_lines, callback) = sink();
    let result = interpret_streaming_with_vfs(src, callback, Some(vfs)).await;
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["hello world".to_string()], "file roundtrip through the VFS");
}

#[tokio::test]
async fn interp_streaming_runs_concurrency_and_streams_live() {
    // The Studio's streaming path must drive concurrent programs through the
    // deterministic scheduler (NOT call `interp.run` directly, which would
    // `yield_request` with no scheduler and panic) — and each line must reach the
    // live callback as it is produced.
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive first from jobs.\n\
        \x20   Receive second from jobs.\n\
        \x20   Show first.\n\
        \x20   Show second.\n";
    let (lines, callback) = sink();
    let result = interpret_streaming_with_vfs(src, callback, None).await;
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["1".to_string(), "2".to_string()], "scheduler-driven result");
    assert_eq!(
        *lines.borrow(),
        vec!["1".to_string(), "2".to_string()],
        "each line reached the live streaming callback"
    );
}

#[tokio::test]
async fn interp_streaming_without_vfs_reports_missing() {
    // Without a VFS, a file write surfaces a clear error rather than silently
    // succeeding — proving the threading is what enables I/O.
    let src = "## Main\n\
        Write \"x\" to file \"out.txt\".\n";
    let (_lines, callback) = sink();
    let result = interpret_streaming_with_vfs(src, callback, None).await;
    let err = result.error.expect("file I/O without a VFS must error");
    assert!(err.contains("VFS"), "error names the missing VFS: {err}");
}

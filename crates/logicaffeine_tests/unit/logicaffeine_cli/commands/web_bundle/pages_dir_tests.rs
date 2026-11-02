//! ★ LOCK — a `pages/` directory that cannot be READ is an error, not an empty site. ★
//!
//! `page_sources` already returns a `Result` and already reports a page that fails to assemble.
//! The DIRECTORY read was the one step that swallowed its own failure: `if let Ok(read) =
//! read_dir(…)` with no else, so an unreadable `pages/` produced zero entries and every caller —
//! `largo build`, `largo check`, `largo serve` — treated that as "this project has no pages".
//!
//! ★ THE FAILURE IS SILENT AND THE ARTIFACT IS WRONG. A build does not stop; it ships a site with
//! every page missing and says nothing, and `largo check` calls that site green. The directory is
//! known to EXIST at this point (callers test `is_dir()` first), so a read that fails is a real
//! fault — permissions, a broken mount, an IO error — never "there is nothing here".
use std::fs;

#[test]
fn an_unreadable_pages_directory_is_reported_rather_than_read_as_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pages = dir.path().join("pages");
    fs::create_dir(&pages).expect("pages/");
    fs::write(pages.join("index.lg"), "## Page /\nView:\n    <p>hi</p>\n").expect("a page");

    // Make it unreadable. If that does not actually block reading — running as root, or a
    // filesystem that ignores the mode — say so rather than passing silently: a green test
    // that proved nothing is how this class of bug survives.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pages, fs::Permissions::from_mode(0o000)).expect("chmod");
    }
    if fs::read_dir(&pages).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&pages, fs::Permissions::from_mode(0o755));
        }
        eprintln!("LOGOS_TOOL_SKIP: this process can read a 0o000 directory (root?)");
        return;
    }

    let got = super::page_sources(&pages);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&pages, fs::Permissions::from_mode(0o755));
    }

    let err = got.err().unwrap_or_else(|| {
        panic!(
            "an unreadable `pages/` must be an ERROR — answering an empty page set makes \
                 `largo build` ship a site with every page missing, and `largo check` call it green"
        )
    });
    assert!(
        err.0.contains("pages"),
        "the message must name the directory it could not read, so the reader knows where to \
             look: {}",
        err.0
    );
}

#[test]
fn a_readable_pages_directory_still_yields_its_pages() {
    // The control. A `page_sources` that errored on EVERYTHING would satisfy the test above
    // while breaking every project that works.
    let dir = tempfile::tempdir().expect("tempdir");
    let pages = dir.path().join("pages");
    fs::create_dir(&pages).expect("pages/");
    fs::write(pages.join("index.lg"), "## Page /\nView:\n    <p>hi</p>\n").expect("a page");
    fs::write(pages.join("notes.txt"), "not a page").expect("a non-page file");

    let got = super::page_sources(&pages).expect("a readable directory reads");
    let stems: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(
        stems,
        vec!["index"],
        "only `.lg` files are pages, and they are sorted by stem"
    );
}

#[test]
fn a_missing_pages_directory_is_no_pages_rather_than_a_failure() {
    // ★ The other half of the pair. A site with `static/` and no `pages/` is ordinary — and
    // the first version of this function failed it, because it treated every `read_dir` error
    // alike. That put an `is_dir()` obligation on every caller, and `plan_site` promptly
    // forgot it. The distinction belongs HERE, where it cannot be forgotten.
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("pages");
    let got = super::page_sources(&missing).expect("a missing pages/ is not an error");
    assert!(got.is_empty(), "and it answers no pages, not a phantom one");
}

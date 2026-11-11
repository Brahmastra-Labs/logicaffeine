//! The section compile cache: a section's passes run once for every compile that hands them the same template.
//!
//! A section is compiled by running the pinned passes over its template, and what the passes make of it is a function
//! of exactly four things: the template, the target, the passes run, and the compiler that runs them. A compile that
//! hands a pass list a template an earlier compile already passed — for the same target, the same passes, with the same
//! compiler — takes the passed section from the cache. What a new batch size, context length or chip changes in a
//! deployment is the templates it changes, and those alone are compiled again; every program is still composed and
//! certified whole.
//!
//! The key is the SHA-256 of the cache's format, the compiler's own executable, the passes by name, the target and the
//! template, each serialized whole — never a summary of one, so no change to any of them reaches another's entry. A pass
//! list that ends in the certifying pass is never cached: a certificate is the proof a compile makes for itself, and an
//! entry carries none. An entry is the passed program serialized, under its key's hex in the cache's directory; one that
//! does not read back is compiled again and written over.
//!
//! The cache is ambient to a compile, as a build directory is: [`scoped`] makes it the one every pass list on the thread
//! consults while the compile runs, and nothing else changes for it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::ir::SpatialProgram;
use super::passes::SpatialReject;
use super::target::SpatialTarget;

/// The layout of a key and an entry; a change to either is a new format, and finds no earlier entry.
const FORMAT: &[u8] = b"logos.spatial.section_cache.v1";

/// The pass whose list a cache never holds: it certifies.
const CERTIFYING: &str = "verify";

/// How the sections a compile asked for were had: passed, or taken from the cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheCounts {
    pub compiled: usize,
    pub reused: usize,
}

impl std::fmt::Display for CacheCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} sections compiled, {} reused", self.compiled, self.reused)
    }
}

/// A compile cache of passed sections: in memory for the compiles that share it, and in a directory when it has one.
pub struct SectionCache {
    dir: Option<PathBuf>,
    /// The compiler's identity: the SHA-256 of the executable running it, for a cache kept on disk.
    compiler: [u8; 32],
    memory: RefCell<HashMap<[u8; 32], SpatialProgram>>,
    counts: Cell<CacheCounts>,
}

impl SectionCache {
    /// A cache kept in `dir` (made when missing) across the compiles this executable runs.
    pub fn in_dir(dir: &Path) -> std::io::Result<SectionCache> {
        std::fs::create_dir_all(dir)?;
        let exe = std::env::current_exe()?;
        let compiler = crate::crypto_accel::sha256(&std::fs::read(&exe)?);
        Ok(SectionCache { dir: Some(dir.to_path_buf()), compiler, memory: RefCell::default(), counts: Cell::default() })
    }

    /// A cache kept in memory, for the compiles that share it.
    pub fn in_memory() -> SectionCache {
        SectionCache { dir: None, compiler: [0; 32], memory: RefCell::default(), counts: Cell::default() }
    }

    /// How the sections were had so far.
    pub fn counts(&self) -> CacheCounts {
        self.counts.get()
    }

    fn count(&self, reused: bool) {
        let mut c = self.counts.get();
        if reused {
            c.reused += 1;
        } else {
            c.compiled += 1;
        }
        self.counts.set(c);
    }

    fn key(&self, p: &SpatialProgram, target: &SpatialTarget, passes: &[&str]) -> [u8; 32] {
        let mut msg = Vec::with_capacity(4096);
        msg.extend_from_slice(FORMAT);
        msg.extend_from_slice(&self.compiler);
        for name in passes {
            msg.extend_from_slice(name.as_bytes());
            msg.push(0);
        }
        msg.extend_from_slice(&crate::crypto_accel::sha256(&serde_json::to_vec(target).expect("a target serializes")));
        serde_json::to_writer(&mut msg, p).expect("a program serializes");
        crate::crypto_accel::sha256(&msg)
    }

    fn entry(&self, key: &[u8; 32]) -> Option<PathBuf> {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        self.dir.as_ref().map(|d| d.join(format!("{hex}.json")))
    }

    /// The section `run` makes of `p` under `passes` on `target`: taken from the cache when an earlier compile made it,
    /// otherwise made, and kept.
    fn through(
        &self,
        p: SpatialProgram,
        target: &SpatialTarget,
        passes: &[&str],
        run: impl FnOnce(SpatialProgram) -> Result<SpatialProgram, SpatialReject>,
    ) -> Result<SpatialProgram, SpatialReject> {
        let key = self.key(&p, target, passes);
        if let Some(hit) = self.memory.borrow().get(&key) {
            self.count(true);
            return Ok(hit.clone());
        }
        let file = self.entry(&key);
        let kept = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|bytes| serde_json::from_slice::<SpatialProgram>(&bytes).ok());
        if let Some(hit) = kept {
            self.count(true);
            self.memory.borrow_mut().insert(key, hit.clone());
            return Ok(hit);
        }
        let made = run(p)?;
        self.count(false);
        if let Some(f) = file {
            let partial = f.with_extension(format!("json.{}", std::process::id()));
            let written = serde_json::to_vec(&made).map_err(std::io::Error::other).and_then(|bytes| std::fs::write(&partial, bytes)).and_then(|_| std::fs::rename(&partial, &f));
            if written.is_err() {
                let _ = std::fs::remove_file(&partial);
            }
        }
        self.memory.borrow_mut().insert(key, made.clone());
        Ok(made)
    }
}

/// `compile` run through the section cache kept in `dir`, when a compile names one: what it made, and the line that says
/// what it compiled and what it reused there.
pub fn compile_through<R>(dir: Option<&Path>, compile: impl FnOnce() -> R) -> std::io::Result<(R, Option<String>)> {
    match dir {
        None => Ok((compile(), None)),
        Some(d) => {
            let cache = Rc::new(SectionCache::in_dir(d)?);
            let made = scoped(cache.clone(), compile);
            Ok((made, Some(format!("compile cache {}: {}", d.display(), cache.counts()))))
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<SectionCache>>> = const { RefCell::new(None) };
}

/// Run `f` with `cache` the compile cache of this thread, the cache it had before restored after.
pub fn scoped<R>(cache: Rc<SectionCache>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<SectionCache>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(CURRENT.with(|c| c.borrow_mut().replace(cache)));
    f()
}

/// `run` over `p` — the pass list `passes` on `target` — through the thread's compile cache, when it has one and the list
/// does not certify.
pub(crate) fn passed(
    p: SpatialProgram,
    target: &SpatialTarget,
    passes: &[&str],
    run: impl FnOnce(SpatialProgram) -> Result<SpatialProgram, SpatialReject>,
) -> Result<SpatialProgram, SpatialReject> {
    let cache = CURRENT.with(|c| c.borrow().clone());
    match cache {
        Some(cache) if !passes.contains(&CERTIFYING) => cache.through(p, target, passes, run),
        _ => run(p),
    }
}

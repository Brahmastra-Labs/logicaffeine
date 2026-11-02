//! Pins of artifacts by digest, one family to a fixture under `tests/fixtures/one_ir_identity/`: the shared half of
//! `one_ir_identity_lock` and `one_ir_lane_identity_lock`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use logicaffeine_compile::crypto_accel::sha256;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest of an artifact's parts, each prefixed by its length so no two splits of the same bytes agree.
pub fn digest(parts: &[&[u8]]) -> String {
    let mut all = Vec::new();
    for p in parts {
        all.extend_from_slice(&(p.len() as u64).to_le_bytes());
        all.extend_from_slice(p);
    }
    hex(&sha256(&all))
}

/// A refusal's pin: its text's digest, marked so a refusal never reads as a program.
pub fn refused(why: &str) -> String {
    format!("refused:{}", digest(&[why.as_bytes()]))
}

/// The pins of one family, by key.
pub type Pins = BTreeMap<String, String>;

/// Run `work` over `items` on the box's cores, at most eight at once, the results merged.
pub fn parallel<T: Sync>(items: &[T], work: impl Fn(&T) -> Pins + Sync) -> Pins {
    let next = Mutex::new(0usize);
    let merged = Mutex::new(Pins::new());
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8).min(items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    let i = *n;
                    *n += 1;
                    i
                };
                let Some(item) = items.get(i) else { break };
                let pins = work(item);
                merged.lock().unwrap().extend(pins);
            });
        }
    });
    merged.into_inner().unwrap()
}

fn fixture(family: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/one_ir_identity/{family}.txt"))
}

/// Every `.rs` file under `dir`, in path order.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The tree a fixture is recorded on: the commit checked out, and the digest of the compiler's sources as they are.
fn tree_state() -> String {
    let git = root().join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).unwrap_or_default();
    let commit = match head.trim().strip_prefix("ref: ") {
        Some(r) => std::fs::read_to_string(git.join(r)).unwrap_or_default().trim().to_string(),
        None => head.trim().to_string(),
    };
    let mut files = Vec::new();
    sources(&root().join("crates/logicaffeine_compile/src"), &mut files);
    let mut all = Vec::new();
    for f in files {
        all.extend_from_slice(f.strip_prefix(root()).unwrap().to_string_lossy().as_bytes());
        all.push(0);
        all.extend(std::fs::read(&f).unwrap());
        all.push(0);
    }
    format!("commit {} with crates/logicaffeine_compile/src at sha256 {}", if commit.is_empty() { "unknown" } else { &commit }, hex(&sha256(&all)))
}

fn render(family: &str, pins: &Pins) -> String {
    let mut s = format!(
        "# one_ir_identity/{family}: {} pins, recorded on {}.\n# Re-record only with LOGOS_UPDATE_FIXTURES=1 and a reason; a migration step never re-records.\n",
        pins.len(),
        tree_state()
    );
    for (k, v) in pins {
        s.push_str(&format!("{k} {v}\n"));
    }
    s
}

pub fn read_pins(family: &str) -> Pins {
    let path = fixture(family);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("{}: {e} — record it once, on the tree before the migration, with LOGOS_UPDATE_FIXTURES=1", path.display())
    });
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let (k, v) = l.rsplit_once(' ').unwrap_or_else(|| panic!("{}: a line is `key digest`: {l}", path.display()));
            (k.to_string(), v.to_string())
        })
        .collect()
}

/// Hold `family`'s pins, as `generate` computes them, to its fixture — or record it, generated twice and alike.
pub fn hold(family: &str, generate: impl Fn(&str) -> Pins) {
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok_and(|v| v == "1") {
        let (first, second) = (generate("a"), generate("b"));
        let differ: Vec<&String> = first.keys().filter(|k| first.get(*k) != second.get(*k)).collect();
        assert!(differ.is_empty(), "{family}: generated twice, these artifacts differ, so they cannot be pinned: {differ:?}");
        assert_eq!(first.len(), second.len(), "{family}: generated twice, the keys differ");
        std::fs::create_dir_all(fixture(family).parent().unwrap()).unwrap();
        std::fs::write(fixture(family), render(family, &first)).unwrap();
        return;
    }
    let want = read_pins(family);
    let got = generate("a");
    let moved: Vec<String> = got.iter().filter(|(k, v)| want.get(*k).is_some_and(|w| w != *v)).map(|(k, _)| k.clone()).collect();
    let new: Vec<&String> = got.keys().filter(|k| !want.contains_key(*k)).collect();
    let gone: Vec<&String> = want.keys().filter(|k| !got.contains_key(*k)).collect();
    let show = |v: &[String]| v.iter().take(40).cloned().collect::<Vec<_>>().join("\n  ");
    assert!(
        moved.is_empty() && new.is_empty() && gone.is_empty(),
        "{family}: the artifacts are not the pinned ones.\n{} moved:\n  {}\n{} not pinned: {:?}\n{} pinned but no longer produced: {:?}\n\
         A migration step changes no artifact. A deliberate backend change re-records its family as it lands \
         (LOGOS_UPDATE_FIXTURES=1 cargo nextest run -p logicaffeine-tests --test one_ir_identity_lock), saying why.",
        moved.len(),
        show(&moved),
        new.len(),
        new.iter().take(40).collect::<Vec<_>>(),
        gone.len(),
        gone.iter().take(40).collect::<Vec<_>>()
    );
}

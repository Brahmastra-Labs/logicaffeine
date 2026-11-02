//! The C family's convergent collections (`codegen_c::CRDT_CORE`) behave exactly as the data crate's do.
//!
//! `SharedSet of T`, `SharedSet (RemoveWins) of T` and `SharedMap from K to V` hold the data crate's
//! `ORSet<CrdtScalar, AddWins>`, `ORSet<CrdtScalar, RemoveWins>` and `ORMap<CrdtScalar, LatestWrite<CrdtScalar>>` on
//! the tree-walker and the VM, and `crdt_core.c` on every C-family tier and RISC-V. This lock runs the same
//! operation scripts — adds, removes, writes, copies, and merges between replicas in every direction — on both,
//! and requires every observation to agree: each collection's contents in order and its length, membership,
//! and a map's value under a key. Seeded scripts over small domains make concurrent adds, removes and
//! equal-stamp writes common; hand-written scripts pin each bias's defining case. The core runs under the
//! address, undefined-behavior and leak sanitizers, so a memory error or a leaked byte fails here too.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use logicaffeine_compile::codegen_c::CRDT_CORE;
use logicaffeine_compile::semantics::crdt::CrdtScalar;
use logicaffeine_data::crdt::{AddWins, LatestWrite, Merge, ORMap, ORSet, RemoveWins};

/// The driver: a tier's qualifiers and allocator, the core, and a reader of the script language below.
const DRIVER: &str = r#"
static void *objs[64];
static int is_map[64], rw[64], ek[64], vk[64];

/* The driver's own text cells: the core copies what it keeps, so every cell the driver made is freed at exit. */
static char **scratch;
static long long nscratch, capscratch;

static void drop_obj(int id) {
    if (!objs[id]) return;
    if (is_map[id]) logos_crdt_map_free((logos_crdt_map *)objs[id]); else logos_crdt_set_free((logos_crdt_set *)objs[id]);
    objs[id] = 0;
}

static int kind_of(const char *w) { return !strcmp(w, "text") ? LOGOS_CRDT_TEXT : !strcmp(w, "bool") ? LOGOS_CRDT_BOOL : LOGOS_CRDT_INT; }

static long long cell(const char *w, int kind) {
    if (kind == LOGOS_CRDT_TEXT) {
        if (nscratch == capscratch) { capscratch = capscratch ? capscratch * 2 : 64; scratch = (char **)realloc(scratch, (size_t)capscratch * sizeof(char *)); }
        return (long long)(intptr_t)(scratch[nscratch++] = strdup(w));
    }
    if (kind == LOGOS_CRDT_BOOL) return !strcmp(w, "true");
    return atoll(w);
}

static void put_cell(long long c, int kind) {
    if (kind == LOGOS_CRDT_TEXT) fputs((const char *)(intptr_t)c, stdout);
    else if (kind == LOGOS_CRDT_BOOL) fputs(c ? "true" : "false", stdout);
    else printf("%lld", c);
}

static void show(int id) {
    printf("%d: {", id);
    if (is_map[id]) {
        logos_crdt_map *m = (logos_crdt_map *)objs[id];
        for (long long i = 0; i < logos_crdt_map_len(m); i++) {
            if (i) fputs(", ", stdout);
            put_cell(logos_crdt_map_key_nth(m, i), ek[id]);
            fputs(": ", stdout);
            put_cell(logos_crdt_map_value_nth(m, i), vk[id]);
        }
        printf("} len=%lld\n", logos_crdt_map_len(m));
    } else {
        logos_crdt_set *s = (logos_crdt_set *)objs[id];
        for (long long i = 0; i < logos_crdt_set_len(s); i++) {
            if (i) fputs(", ", stdout);
            put_cell(logos_crdt_set_nth(s, i), ek[id]);
        }
        printf("} len=%lld\n", logos_crdt_set_len(s));
    }
}

int main(void) {
    char op[16], a[64], b[64], c[64];
    while (scanf("%15s", op) == 1) {
        if (!strcmp(op, "newset")) {
            int id; scanf("%d %63s %63s", &id, a, b);
            drop_obj(id);
            is_map[id] = 0; rw[id] = !strcmp(a, "rw"); ek[id] = kind_of(b);
            objs[id] = logos_crdt_set_new(ek[id], rw[id]);
        } else if (!strcmp(op, "newmap")) {
            int id; scanf("%d %63s %63s", &id, a, b);
            drop_obj(id);
            is_map[id] = 1; ek[id] = kind_of(a); vk[id] = kind_of(b);
            objs[id] = logos_crdt_map_new(ek[id], vk[id]);
        } else if (!strcmp(op, "add")) {
            int id; scanf("%d %63s", &id, a);
            logos_crdt_set_add((logos_crdt_set *)objs[id], cell(a, ek[id]));
        } else if (!strcmp(op, "rem")) {
            int id; scanf("%d %63s", &id, a);
            logos_crdt_set_remove((logos_crdt_set *)objs[id], cell(a, ek[id]));
        } else if (!strcmp(op, "put")) {
            int id; scanf("%d %63s %63s", &id, a, b);
            logos_crdt_map_put((logos_crdt_map *)objs[id], cell(a, ek[id]), cell(b, vk[id]));
        } else if (!strcmp(op, "del")) {
            int id; scanf("%d %63s", &id, a);
            logos_crdt_map_remove((logos_crdt_map *)objs[id], cell(a, ek[id]));
        } else if (!strcmp(op, "merge")) {
            int d, s; scanf("%d %d", &d, &s);
            if (is_map[d]) logos_crdt_map_merge((logos_crdt_map *)objs[d], (logos_crdt_map *)objs[s]);
            else logos_crdt_set_merge((logos_crdt_set *)objs[d], (logos_crdt_set *)objs[s]);
        } else if (!strcmp(op, "copy")) {
            int d, s; scanf("%d %d", &d, &s);
            void *copy = is_map[s] ? (void *)logos_crdt_map_clone((logos_crdt_map *)objs[s]) : (void *)logos_crdt_set_clone((logos_crdt_set *)objs[s]);
            drop_obj(d);
            is_map[d] = is_map[s]; rw[d] = rw[s]; ek[d] = ek[s]; vk[d] = vk[s];
            objs[d] = copy;
        } else if (!strcmp(op, "show")) {
            int id; scanf("%d", &id);
            show(id);
        } else if (!strcmp(op, "has")) {
            int id; scanf("%d %63s", &id, a);
            int yes = is_map[id] ? logos_crdt_map_contains((logos_crdt_map *)objs[id], cell(a, ek[id]))
                                 : logos_crdt_set_contains((logos_crdt_set *)objs[id], cell(a, ek[id]));
            printf("%d has %s: %s\n", id, a, yes ? "true" : "false");
        } else if (!strcmp(op, "get")) {
            int id, present; scanf("%d %63s", &id, a);
            long long v = logos_crdt_map_get((logos_crdt_map *)objs[id], cell(a, ek[id]), &present);
            printf("%d[%s] ", id, a);
            if (present) { fputs("= ", stdout); put_cell(v, vk[id]); } else fputs("missing", stdout);
            putchar('\n');
        }
        (void)c;
    }
    for (int id = 0; id < 64; id++) drop_obj(id);
    for (long long i = 0; i < nscratch; i++) free(scratch[i]);
    free(scratch);
    return 0;
}
"#;

/// The C side: the core compiled with the driver, run on `script`.
struct CCore {
    bin: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl CCore {
    fn build() -> CCore {
        let dir = tempfile::tempdir().unwrap();
        let src = format!(
            "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n#include <stdint.h>\n\
             #define LOGOS_FN static __attribute__((unused))\n#define LOGOS_VAR static\n\
             #define logos_alloc(n) calloc(1, (n))\n#define logos_release(p) free(p)\n{CRDT_CORE}\n{DRIVER}"
        );
        std::fs::write(dir.path().join("core.c"), src).unwrap();
        let bin = dir.path().join("core");
        let out = std::process::Command::new("cc")
            .args([
                "-O1",
                "-g",
                "-Wall",
                "-Werror",
                "-Wno-unused-result",
                "-fsanitize=address,undefined",
                "-fno-sanitize-recover=all",
                "-o",
            ])
            .arg(&bin)
            .arg(dir.path().join("core.c"))
            .output()
            .expect("run cc");
        assert!(
            out.status.success(),
            "crdt_core.c does not build:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        CCore { bin, _dir: dir }
    }

    fn run(&self, script: &str) -> String {
        use std::io::Write as _;
        let mut child = std::process::Command::new(&self.bin)
            .env("ASAN_OPTIONS", "detect_leaks=1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "the C core's driver failed ({:?}) on:\n{script}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
}

/// One collection on the data crate's side.
enum Obj {
    AddWins(ORSet<CrdtScalar, AddWins>, Kind),
    RemoveWins(ORSet<CrdtScalar, RemoveWins>, Kind),
    Map(ORMap<CrdtScalar, LatestWrite<CrdtScalar>>, Kind, Kind),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Int,
    Text,
    Bool,
}

impl Kind {
    fn parse(w: &str) -> Kind {
        match w {
            "text" => Kind::Text,
            "bool" => Kind::Bool,
            _ => Kind::Int,
        }
    }

    fn cell(self, w: &str) -> CrdtScalar {
        match self {
            Kind::Text => CrdtScalar::Text(w.to_string()),
            Kind::Bool => CrdtScalar::Bool(w == "true"),
            Kind::Int => CrdtScalar::Int(w.parse().unwrap()),
        }
    }
}

fn render(v: &CrdtScalar) -> String {
    v.render()
}

/// The data crate's side of a script, printing what the driver prints.
fn run_rust(script: &str) -> String {
    let mut objs: BTreeMap<usize, Obj> = BTreeMap::new();
    let mut next_replica = 1u64;
    let mut out = String::new();
    let mut words = script.split_whitespace();
    while let Some(op) = words.next() {
        let mut w = || words.next().unwrap();
        match op {
            "newset" => {
                let (id, bias, kind): (usize, &str, &str) = (w().parse().unwrap(), w(), w());
                let replica = next_replica;
                next_replica += 1;
                let k = Kind::parse(kind);
                objs.insert(
                    id,
                    if bias == "rw" {
                        Obj::RemoveWins(ORSet::new(replica), k)
                    } else {
                        Obj::AddWins(ORSet::new(replica), k)
                    },
                );
            }
            "newmap" => {
                let (id, kk, vk): (usize, &str, &str) = (w().parse().unwrap(), w(), w());
                let replica = next_replica;
                next_replica += 1;
                objs.insert(
                    id,
                    Obj::Map(ORMap::new(replica), Kind::parse(kk), Kind::parse(vk)),
                );
            }
            "add" | "rem" => {
                let (id, v): (usize, &str) = (w().parse().unwrap(), w());
                match objs.get_mut(&id).unwrap() {
                    Obj::AddWins(s, k) => {
                        let e = k.cell(v);
                        if op == "add" {
                            s.add(e)
                        } else {
                            s.remove(&e)
                        }
                    }
                    Obj::RemoveWins(s, k) => {
                        let e = k.cell(v);
                        if op == "add" {
                            s.add(e)
                        } else {
                            s.remove(&e)
                        }
                    }
                    Obj::Map(..) => unreachable!(),
                }
            }
            "put" => {
                let (id, key, v): (usize, &str, &str) = (w().parse().unwrap(), w(), w());
                if let Obj::Map(m, kk, vk) = objs.get_mut(&id).unwrap() {
                    let (key, v) = (kk.cell(key), vk.cell(v));
                    m.put(key, v);
                }
            }
            "del" => {
                let (id, key): (usize, &str) = (w().parse().unwrap(), w());
                if let Obj::Map(m, kk, _) = objs.get_mut(&id).unwrap() {
                    m.remove(&kk.cell(key));
                }
            }
            "merge" => {
                let (d, s): (usize, usize) = (w().parse().unwrap(), w().parse().unwrap());
                let source = match objs.get(&s).unwrap() {
                    Obj::AddWins(x, k) => Obj::AddWins(x.clone(), *k),
                    Obj::RemoveWins(x, k) => Obj::RemoveWins(x.clone(), *k),
                    Obj::Map(x, k, v) => Obj::Map(x.clone(), *k, *v),
                };
                match (objs.get_mut(&d).unwrap(), &source) {
                    (Obj::AddWins(a, _), Obj::AddWins(b, _)) => a.merge(b),
                    (Obj::RemoveWins(a, _), Obj::RemoveWins(b, _)) => a.merge(b),
                    (Obj::Map(a, ..), Obj::Map(b, ..)) => a.merge(b),
                    _ => unreachable!("scripts merge like with like"),
                }
            }
            "copy" => {
                let (d, s): (usize, usize) = (w().parse().unwrap(), w().parse().unwrap());
                let copy = match objs.get(&s).unwrap() {
                    Obj::AddWins(x, k) => Obj::AddWins(x.clone(), *k),
                    Obj::RemoveWins(x, k) => Obj::RemoveWins(x.clone(), *k),
                    Obj::Map(x, k, v) => Obj::Map(x.clone(), *k, *v),
                };
                objs.insert(d, copy);
            }
            "show" => {
                let id: usize = w().parse().unwrap();
                let (body, len) = match objs.get(&id).unwrap() {
                    Obj::AddWins(s, _) => {
                        let mut e: Vec<&CrdtScalar> = s.iter().collect();
                        e.sort();
                        (
                            e.iter().map(|v| render(v)).collect::<Vec<_>>().join(", "),
                            s.len(),
                        )
                    }
                    Obj::RemoveWins(s, _) => {
                        let mut e: Vec<&CrdtScalar> = s.iter().collect();
                        e.sort();
                        (
                            e.iter().map(|v| render(v)).collect::<Vec<_>>().join(", "),
                            s.len(),
                        )
                    }
                    Obj::Map(m, ..) => (
                        m.sorted_entries()
                            .iter()
                            .map(|(k, v)| format!("{}: {}", render(k), render(v.get())))
                            .collect::<Vec<_>>()
                            .join(", "),
                        m.len(),
                    ),
                };
                writeln!(out, "{id}: {{{body}}} len={len}").unwrap();
            }
            "has" => {
                let (id, v): (usize, &str) = (w().parse().unwrap(), w());
                let yes = match objs.get(&id).unwrap() {
                    Obj::AddWins(s, k) => s.contains(&k.cell(v)),
                    Obj::RemoveWins(s, k) => s.contains(&k.cell(v)),
                    Obj::Map(m, k, _) => m.contains_key(&k.cell(v)),
                };
                writeln!(out, "{id} has {v}: {yes}").unwrap();
            }
            "get" => {
                let (id, key): (usize, &str) = (w().parse().unwrap(), w());
                if let Obj::Map(m, k, _) = objs.get(&id).unwrap() {
                    match m.value(&k.cell(key)) {
                        Some(v) => writeln!(out, "{id}[{key}] = {}", render(v)).unwrap(),
                        None => writeln!(out, "{id}[{key}] missing").unwrap(),
                    }
                }
            }
            other => panic!("unknown script op `{other}`"),
        }
    }
    out
}

/// A seeded script over one collection type: `objects` replicas, `steps` operations drawn from small domains,
/// every replica shown at the end.
fn script(seed: u64, steps: usize) -> String {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut rnd = move |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    let shape = rnd(6);
    let (new, is_map, key_kind, value_kind) = match shape {
        0 => ("newset {} aw text", false, "text", ""),
        1 => ("newset {} rw text", false, "text", ""),
        2 => ("newset {} aw int", false, "int", ""),
        3 => ("newset {} rw int", false, "int", ""),
        4 => ("newmap {} text int", true, "text", "int"),
        _ => ("newmap {} int text", true, "int", "text"),
    };
    let texts = ["a", "b", "c", "d", "e"];
    let pick = |kind: &str, r: u64| -> String {
        match kind {
            "text" => texts[(r % 5) as usize].to_string(),
            _ => ((r % 6) as i64 - 2).to_string(),
        }
    };
    let mut s = String::new();
    let mut live = 0usize;
    for _ in 0..2 {
        s += &new.replace("{}", &live.to_string());
        s.push('\n');
        live += 1;
    }
    for _ in 0..steps {
        let id = rnd(live as u64) as usize;
        match rnd(10) {
            0 if live < 8 => {
                s += &new.replace("{}", &live.to_string());
                s.push('\n');
                live += 1;
            }
            1 if live < 8 => {
                writeln!(s, "copy {live} {id}").unwrap();
                live += 1;
            }
            2 | 3 => {
                let other = rnd(live as u64) as usize;
                if other != id {
                    writeln!(s, "merge {id} {other}").unwrap();
                }
            }
            4 if !is_map => writeln!(s, "rem {id} {}", pick(key_kind, rnd(1000))).unwrap(),
            4 => writeln!(s, "del {id} {}", pick(key_kind, rnd(1000))).unwrap(),
            5 => writeln!(s, "show {id}").unwrap(),
            6 => writeln!(s, "has {id} {}", pick(key_kind, rnd(1000))).unwrap(),
            7 if is_map => writeln!(s, "get {id} {}", pick(key_kind, rnd(1000))).unwrap(),
            _ if is_map => writeln!(
                s,
                "put {id} {} {}",
                pick(key_kind, rnd(1000)),
                pick(value_kind, rnd(1000))
            )
            .unwrap(),
            _ => writeln!(s, "add {id} {}", pick(key_kind, rnd(1000))).unwrap(),
        }
    }
    for id in 0..live {
        writeln!(s, "show {id}").unwrap();
    }
    s
}

fn assert_agree(core: &CCore, label: &str, script: &str) {
    let (c, rust) = (core.run(script), run_rust(script));
    if c != rust {
        let first = c
            .lines()
            .zip(rust.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(c.lines().count().min(rust.lines().count()));
        panic!(
            "`{label}`: the C core and the data crate disagree at observation {first}:\n  C:    {:?}\n  Rust: {:?}\n\nscript:\n{script}",
            c.lines().nth(first),
            rust.lines().nth(first)
        );
    }
}

/// Each bias's defining case, a concurrent write tie, and the shapes a merge meets: a remove the other replica
/// observed, a remove it did not, a re-add after a remove, and a key written, removed and written again.
const PINNED: &[(&str, &str)] = &[
    (
        "add-wins: a concurrent add beats a remove",
        "newset 0 aw text\nadd 0 x\ncopy 1 0\nnewset 2 aw text\nmerge 2 0\nrem 0 x\nadd 2 x\nmerge 0 2\nshow 0\nhas 0 x\nmerge 1 0\nshow 1\n",
    ),
    (
        "remove-wins: a concurrent remove beats an add",
        "newset 0 rw text\nnewset 1 rw text\nadd 0 x\nmerge 1 0\nrem 1 x\nadd 0 x\nmerge 0 1\nshow 0\nhas 0 x\n",
    ),
    (
        "an observed remove survives a merge back",
        "newset 0 aw int\nnewset 1 aw int\nadd 0 1\nadd 0 2\nmerge 1 0\nrem 1 1\nmerge 0 1\nshow 0\nshow 1\n",
    ),
    (
        "a re-add after a remove is a new dot",
        "newset 0 aw text\nnewset 1 aw text\nadd 0 k\nmerge 1 0\nrem 0 k\nadd 0 k\nmerge 1 0\nshow 1\nmerge 0 1\nshow 0\n",
    ),
    (
        "equal stamps: the larger value wins on both replicas",
        "newmap 0 text int\nnewmap 1 text int\nput 0 k 5\nput 1 k 9\nmerge 0 1\nmerge 1 0\nshow 0\nshow 1\nget 0 k\n",
    ),
    (
        "a put concurrent with a remove keeps the key",
        "newmap 0 text int\nput 0 k 1\nnewmap 1 text int\nmerge 1 0\ndel 1 k\nput 0 k 2\nmerge 1 0\nshow 1\nget 1 k\n",
    ),
    (
        "a key written, removed and written again",
        "newmap 0 int text\nput 0 3 a\ndel 0 3\nget 0 3\nput 0 3 b\nget 0 3\nnewmap 1 int text\nmerge 1 0\nshow 1\n",
    ),
];

#[test]
fn the_c_core_merges_as_the_data_crate_does() {
    if !common::require_tool("cc") {
        return;
    }
    let core = CCore::build();
    for (label, script) in PINNED {
        assert_agree(&core, label, script);
    }
    for seed in 1..=600u64 {
        assert_agree(&core, &format!("seed {seed}"), &script(seed, 120));
    }
}

//! The pedagogy lock battery: the Syntax Guide teaches the way the masters
//! taught, and every teaching-craft dimension is locked here — hooks before
//! mechanism (Meyer/Dewey), behavioral objectives (Bloom/Mager), confronted
//! misconceptions with runnable evidence (Muller), predict-before-reveal,
//! worked→completion→free exercise fading (Sweller/Renkl), progressive hints
//! that never spoil (Bloom's tutor), retrieval within the section
//! (Roediger & Karpicke) and interleaved checkpoints at part boundaries
//! (Rohrer), exactly-one mental-model box per hard abstraction (Feynman),
//! and SM-2 scheduling over it all.
//!
//! Floors start at campaign-day-zero truth and only rise; style locks are
//! exhaustive over whatever exists from day one, so quality is enforced the
//! moment content lands.

#![cfg(feature = "web-tests")]

use logicaffeine_web::guide_review::{
    due_guide_cards, exercise_key, recall_key, record_exercise_result, record_recall_mark,
    GuideCard, GUIDE_EX_PREFIX, GUIDE_RECALL_PREFIX,
};
use logicaffeine_web::progress::UserProgress;
use logicaffeine_web::ui::pages::guide::content::{
    ExerciseKind, Runnability, HOLE_MARKER, SECTIONS,
};

#[path = "web_common/guide_catalog.rs"]
mod guide_catalog;
use guide_catalog::{CHECKPOINT_SECTIONS, LANGUAGE_SURFACE, NON_TEACHING};

// ─── Raise-only floors (campaign ratchets; each wave raises RED, authors GREEN) ───

/// Teaching sections carrying a non-empty hook.
const HOOK_FLOOR: usize = 29;
/// Sections carrying objectives.
const OBJECTIVE_SECTIONS_FLOOR: usize = 38;
/// Misconception ledger entries across the guide.
const MISCONCEPTION_FLOOR: usize = 19;
/// Examples flagged predict-first.
const PREDICT_FLOOR: usize = 21;
/// Recap questions across the guide.
const RECAP_FLOOR: usize = 126;
/// Exercises, whole guide (mirrors guide_delight's EXERCISE_FLOOR on purpose —
/// same number, different failure message).
const GLOBAL_EXERCISE_FLOOR: usize = 66;
/// Exercises per part (exact part strings). A pile-up in one part can never
/// satisfy another part's requirement.
const EXERCISE_FLOOR_BY_PART: &[(&str, usize)] = &[
    ("Part I: First Steps", 8),
    ("Part II: The Language", 24),
    ("Part III: Systems Programming", 11),
    ("Part IV: Distributed Programming", 7),
    ("Part V: Projects and Tooling", 5),
    ("Part VI: Logic and Verification", 8),
    ("Part VII: Practice", 3),
    ("Part VIII: Reference", 0),
];

/// The ten sections whose abstraction demands a mental-model box (Feynman).
/// `MENTAL_MODEL_DONE` is the ratchet: a section moves here the wave its box
/// is authored; W8 requires DONE == REQUIRED.
const MENTAL_MODEL_REQUIRED: &[&str] = &[
    "closures",
    "generics",
    "memory-ownership",
    "zones",
    "machine-words",
    "concurrency",
    "crdt",
    "logic-mode",
    "proofs",
    "execution-tiers",
];
const MENTAL_MODEL_DONE: &[&str] = &[
    "closures",
    "generics",
    "memory-ownership",
    "zones",
    "machine-words",
    "concurrency",
    "crdt",
    "logic-mode",
    "proofs",
    "execution-tiers",
];

/// Multi-stage builds that must close with a `> **Look back:**` block (Polya).
/// Ratchet like MENTAL_MODEL_DONE; W7 fills it.
const LOOK_BACK_DONE: &[&str] = &["tutorial-project", "complete-examples", "recipes"];

const MENTAL_MODEL_MARKER: &str = "**Mental model:**";
const LOOK_BACK_MARKER: &str = "**Look back:**";

/// Objectives must start with an observable Bloom verb — if you cannot name
/// the behavior, you have not decided what to teach.
const BLOOM_VERBS: &[&str] = &[
    "build",
    "choose",
    "clamp",
    "combine",
    "compute",
    "convert",
    "count",
    "decide",
    "diagnose",
    "distinguish",
    "explain",
    "extend",
    "fix",
    "format",
    "identify",
    "merge",
    "modify",
    "name",
    "order",
    "predict",
    "prove",
    "read",
    "run",
    "spot",
    "split",
    "state",
    "trace",
    "translate",
    "use",
    "write",
];
const BANNED_OBJECTIVE_WORDS: &[&str] = &["understand", "know", "appreciate", "grasp"];
const BANNED_OBJECTIVE_PHRASES: &[&str] = &["learn about", "be familiar"];

/// A hook opens with the problem, never with a tour announcement.
const BANNED_HOOK_OPENINGS: &[&str] = &[
    "This section",
    "In this section",
    "We will",
    "Now that",
    "Let's look at",
    "Welcome",
];

/// Respect is pedagogy (Montessori): the guide never talks down. Extends the
/// prose charter's banned words; scanned over teaching text, never code.
const BANNED_TONE_WORDS: &[&str] = &[
    "simply",
    "just",
    "easy",
    "easily",
    "obviously",
    "powerful",
    "blazingly",
];
const BANNED_TONE_PHRASES: &[&str] = &["of course", "as you can see", "don't worry", "great job"];

// ─── helpers ───

fn is_checkpoint(id: &str) -> bool {
    CHECKPOINT_SECTIONS.contains(&id)
}

fn is_non_teaching(id: &str) -> bool {
    NON_TEACHING.contains(&id) || is_checkpoint(id)
}

/// Word-boundary containment: `needle` appears in `haystack` (case-insensitive)
/// not embedded inside a longer alphanumeric run.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let hay = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(&needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Every teaching-voice string in a section (prose and pedagogy fields —
/// deliberately NOT example/exercise code, which prints what it prints).
fn teaching_text(
    section: &logicaffeine_web::ui::pages::guide::content::Section,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    out.push(("content".into(), section.content.to_string()));
    if !section.hook.is_empty() {
        out.push(("hook".into(), section.hook.to_string()));
    }
    for o in section.objectives {
        out.push(("objective".into(), (*o).to_string()));
    }
    for m in section.misconceptions {
        out.push((
            format!("misconception `{}`", m.id),
            format!("{} {}", m.expectation, m.reality),
        ));
    }
    for r in section.recap {
        out.push((
            format!("recap `{}`", r.id),
            format!("{} {}", r.question, r.answer),
        ));
    }
    for e in section.exercises {
        out.push((format!("exercise `{}` prompt", e.id), e.prompt.to_string()));
        for h in e.hints {
            out.push((format!("exercise `{}` hint", e.id), (*h).to_string()));
        }
    }
    out
}

// ─── P1: hooks ───

#[test]
fn hooks_open_with_a_problem() {
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        if s.hook.is_empty() {
            continue;
        }
        let hook = s.hook.trim();
        if hook.chars().count() < 40 || hook.chars().count() > 400 {
            wrong.push(format!(
                "[{}] hook length {} chars — must be 40..=400 (a headache, not an essay)",
                s.id,
                hook.chars().count()
            ));
        }
        if hook.contains('#') {
            wrong.push(format!("[{}] hook contains a markdown heading", s.id));
        }
        for opening in BANNED_HOOK_OPENINGS {
            if hook.starts_with(opening) {
                wrong.push(format!(
                    "[{}] hook opens with {opening:?} — open with the problem, not the tour",
                    s.id
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nhook violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn hook_coverage_never_regresses() {
    let hooked = SECTIONS
        .iter()
        .filter(|s| !is_non_teaching(s.id) && !s.hook.is_empty())
        .count();
    assert!(
        hooked >= HOOK_FLOOR,
        "teaching sections with hooks fell below the floor ({hooked} < {HOOK_FLOOR})"
    );
    // Endgame form (W8): EVERY teaching section opens with its headache — a new
    // section ships hook-first or not at all.
    let hookless: Vec<&str> = SECTIONS
        .iter()
        .filter(|s| !is_non_teaching(s.id) && s.hook.is_empty())
        .map(|s| s.id)
        .collect();
    assert!(
        hookless.is_empty(),
        "\nteaching section(s) without a hook — the headache comes before the aspirin:\n{hookless:?}\n"
    );
}

// ─── P2: objectives ───

#[test]
fn objectives_are_behavioral() {
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        if s.objectives.is_empty() {
            continue;
        }
        if s.objectives.len() < 2 || s.objectives.len() > 5 {
            wrong.push(format!(
                "[{}] {} objectives — must be 2..=5 (deciding what to teach means choosing)",
                s.id,
                s.objectives.len()
            ));
        }
        for o in s.objectives {
            if o.chars().count() > 120 {
                wrong.push(format!("[{}] objective too long (>120 chars): {o:?}", s.id));
            }
            let first = o.split_whitespace().next().unwrap_or("");
            if !BLOOM_VERBS.contains(&first.to_lowercase().as_str()) {
                wrong.push(format!(
                    "[{}] objective must start with a Bloom verb from the allowlist, got {first:?}: {o:?}",
                    s.id
                ));
            }
            for w in BANNED_OBJECTIVE_WORDS {
                if contains_word(o, w) {
                    wrong.push(format!(
                        "[{}] objective contains banned verb {w:?} — name the observable behavior: {o:?}",
                        s.id
                    ));
                }
            }
            for p in BANNED_OBJECTIVE_PHRASES {
                if o.to_lowercase().contains(p) {
                    wrong.push(format!(
                        "[{}] objective contains banned phrase {p:?}: {o:?}",
                        s.id
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nobjective violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn objective_coverage_never_regresses() {
    let covered = SECTIONS.iter().filter(|s| !s.objectives.is_empty()).count();
    assert!(
        covered >= OBJECTIVE_SECTIONS_FLOOR,
        "sections with objectives fell below the floor ({covered} < {OBJECTIVE_SECTIONS_FLOOR})"
    );
    // Endgame form (W8): every section — including reference surfaces — states
    // what the reader can DO afterwards. Deciding what to teach means choosing.
    let missing: Vec<&str> = SECTIONS
        .iter()
        .filter(|s| s.objectives.is_empty())
        .map(|s| s.id)
        .collect();
    assert!(
        missing.is_empty(),
        "\nsection(s) without objectives:\n{missing:?}\n"
    );
}

// ─── P3/P4: misconceptions + predict-first ───

#[test]
fn every_misconception_is_backed_by_runnable_evidence() {
    let mut wrong: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in SECTIONS.iter() {
        for m in s.misconceptions {
            if !seen.insert(m.id) {
                wrong.push(format!("duplicate misconception id `{}`", m.id));
            }
            if m.expectation.trim().is_empty() {
                wrong.push(format!("`{}`: empty expectation", m.id));
            }
            if m.reality.trim().chars().count() < 40 {
                wrong.push(format!(
                    "`{}`: reality must carry the WHY (≥40 chars), got {:?}",
                    m.id, m.reality
                ));
            }
            match s.examples.iter().find(|e| e.id == m.evidence) {
                None => wrong.push(format!(
                    "`{}`: evidence example `{}` not found in section [{}] — misconceptions are \
                     confronted where they arise",
                    m.id, m.evidence, s.id
                )),
                Some(e) => {
                    let is_error = matches!(e.runnability, Runnability::ExpectedError { .. });
                    if !e.predict_first && !is_error {
                        wrong.push(format!(
                            "`{}`: evidence `{}` must be predict-first or ExpectedError — an \
                             unpredicted proof changes no minds (Muller)",
                            m.id, e.id
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nmisconception violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn misconception_count_never_regresses() {
    let count: usize = SECTIONS.iter().map(|s| s.misconceptions.len()).sum();
    assert!(
        count >= MISCONCEPTION_FLOOR,
        "misconception ledger fell below the floor ({count} < {MISCONCEPTION_FLOOR})"
    );
}

#[test]
fn predict_examples_have_something_to_predict() {
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        for e in s.examples.iter() {
            if !e.predict_first {
                continue;
            }
            let ok = match e.runnability {
                Runnability::Playground => e.expected_output.is_some(),
                Runnability::ExpectedError { .. } => true,
                Runnability::CompiledOnly { .. } => false,
            };
            if !ok {
                wrong.push(format!(
                    "`{}` is predict-first without a locked expectation — predicting the \
                     unfalsifiable teaches nothing",
                    e.id
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\npredict-first violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn predict_count_never_regresses() {
    let count = SECTIONS
        .iter()
        .flat_map(|s| s.examples.iter())
        .filter(|e| e.predict_first)
        .count();
    assert!(
        count >= PREDICT_FLOOR,
        "predict-first examples fell below the floor ({count} < {PREDICT_FLOOR})"
    );
}

// ─── P5: worked → completion → free fading ───

#[test]
fn completion_problems_are_true_fades() {
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        for e in s.exercises.iter() {
            let starter_holes = e
                .starter
                .lines()
                .filter(|l| l.contains(HOLE_MARKER))
                .count();
            if e.reference.contains(HOLE_MARKER) {
                wrong.push(format!(
                    "`{}`: the reference solution still contains {HOLE_MARKER}",
                    e.id
                ));
            }
            match e.kind {
                ExerciseKind::Completion => {
                    if starter_holes == 0 {
                        wrong.push(format!(
                            "`{}`: Completion starter has no {HOLE_MARKER} hole",
                            e.id
                        ));
                    }
                    if starter_holes > 3 {
                        wrong.push(format!(
                            "`{}`: {starter_holes} holes — more than 3 is a free exercise in a costume",
                            e.id
                        ));
                    }
                    let reference_lines: std::collections::HashSet<&str> =
                        e.reference.lines().map(str::trim).collect();
                    for line in e.starter.lines() {
                        let t = line.trim();
                        if t.is_empty() || line.contains(HOLE_MARKER) {
                            continue;
                        }
                        if !reference_lines.contains(t) {
                            wrong.push(format!(
                                "`{}`: starter line {t:?} is not in the reference — a Completion \
                                 is the worked solution minus its holes, nothing else",
                                e.id
                            ));
                        }
                    }
                }
                ExerciseKind::Practice | ExerciseKind::Challenge => {
                    if starter_holes > 0 {
                        wrong.push(format!(
                            "`{}`: non-Completion starter contains {HOLE_MARKER}",
                            e.id
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nfading violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn exercise_ladder_fades_in_order() {
    fn rank(k: ExerciseKind) -> u8 {
        match k {
            ExerciseKind::Completion => 0,
            ExerciseKind::Practice => 1,
            ExerciseKind::Challenge => 2,
        }
    }
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        let ranks: Vec<u8> = s.exercises.iter().map(|e| rank(e.kind)).collect();
        if ranks.windows(2).any(|w| w[0] > w[1]) {
            wrong.push(format!(
                "[{}] exercise ladder re-inflates support: kinds must run Completion → Practice \
                 → Challenge (Renkl fading), got {ranks:?}",
                s.id
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "\nladder violation(s):\n{}\n",
        wrong.join("\n")
    );
}

// ─── P11: hints ───

#[test]
fn exercise_hints_orient_and_never_spoil() {
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        for e in s.exercises.iter() {
            if e.hints.is_empty() || e.hints.len() > 3 {
                wrong.push(format!(
                    "`{}`: {} hints — every exercise ships 1..=3 progressive hints (Bloom's tutor)",
                    e.id,
                    e.hints.len()
                ));
            }
            let starter_lines: std::collections::HashSet<&str> =
                e.starter.lines().map(str::trim).collect();
            let distinguishing: Vec<&str> = e
                .reference
                .lines()
                .map(str::trim)
                .filter(|l| l.chars().count() >= 8 && !starter_lines.contains(l))
                .collect();
            for h in e.hints {
                if h.trim().is_empty() {
                    wrong.push(format!("`{}`: empty hint", e.id));
                }
                for line in &distinguishing {
                    if h.contains(line) {
                        wrong.push(format!(
                            "`{}`: hint spoils the answer — it contains the reference line {line:?}",
                            e.id
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nhint violation(s):\n{}\n",
        wrong.join("\n")
    );
}

// ─── exercise concepts + floors ───

#[test]
fn exercise_concepts_are_cataloged_and_scoped() {
    let features: std::collections::HashSet<&str> =
        LANGUAGE_SURFACE.iter().map(|c| c.feature).collect();
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        for e in s.exercises.iter() {
            for c in e.concepts {
                if !features.contains(c) {
                    wrong.push(format!(
                        "`{}`: concept {c:?} is not a LANGUAGE_SURFACE feature",
                        e.id
                    ));
                }
            }
            let n = e.concepts.len();
            let ok = match e.kind {
                ExerciseKind::Completion => n == 1,
                ExerciseKind::Practice => (1..=2).contains(&n),
                ExerciseKind::Challenge => n >= 1,
            };
            if !ok {
                wrong.push(format!(
                    "`{}`: {:?} exercise declares {n} concepts — Completion targets exactly 1, \
                     Practice 1..=2, Challenge ≥1 (deliberate practice targets the hard part)",
                    e.id, e.kind
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nconcept violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn exercise_floors_never_regress() {
    let total: usize = SECTIONS.iter().map(|s| s.exercises.len()).sum();
    assert!(
        total >= GLOBAL_EXERCISE_FLOOR,
        "guide exercise count fell below the floor ({total} < {GLOBAL_EXERCISE_FLOOR})"
    );
    let mut by_part: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for s in SECTIONS.iter() {
        *by_part.entry(s.part).or_insert(0) += s.exercises.len();
    }
    let mut wrong: Vec<String> = Vec::new();
    for (part, floor) in EXERCISE_FLOOR_BY_PART {
        let have = by_part.get(part).copied().unwrap_or(0);
        if have < *floor {
            wrong.push(format!("{part:?}: {have} exercises < floor {floor}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "\nper-part exercise floor violation(s):\n{}\n",
        wrong.join("\n")
    );
}

// ─── P6: recap questions ───

#[test]
fn recap_questions_are_sound() {
    let mut wrong: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let reserved: std::collections::HashSet<&str> = SECTIONS
        .iter()
        .flat_map(|s| {
            std::iter::once(s.id)
                .chain(s.examples.iter().map(|e| e.id))
                .chain(s.exercises.iter().map(|e| e.id))
        })
        .collect();
    for s in SECTIONS.iter() {
        let (lo, hi) = if is_checkpoint(s.id) { (6, 10) } else { (0, 4) };
        if s.recap.len() < lo || s.recap.len() > hi {
            wrong.push(format!(
                "[{}] {} recap questions — must be {lo}..={hi} (retrieval serves the objectives; \
                 more is friction cosplaying as rigor)",
                s.id,
                s.recap.len()
            ));
        }
        for r in s.recap {
            if !seen.insert(r.id) {
                wrong.push(format!("duplicate recap id `{}`", r.id));
            }
            if reserved.contains(r.id) {
                wrong.push(format!(
                    "recap id `{}` collides with a section/example/exercise id",
                    r.id
                ));
            }
            if !r.question.trim().ends_with('?') {
                wrong.push(format!(
                    "`{}`: a recap question ends with `?`: {:?}",
                    r.id, r.question
                ));
            }
            if r.answer.trim().is_empty() || r.answer.chars().count() > 300 {
                wrong.push(format!(
                    "`{}`: recap answer must be a non-empty flashcard back (≤300 chars)",
                    r.id
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nrecap violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn recap_count_never_regresses() {
    let count: usize = SECTIONS.iter().map(|s| s.recap.len()).sum();
    assert!(
        count >= RECAP_FLOOR,
        "recap question count fell below the floor ({count} < {RECAP_FLOOR})"
    );
}

// ─── P7: checkpoints interleave ───

#[test]
fn checkpoints_interleave_earlier_concepts() {
    let position: std::collections::HashMap<&str, usize> = SECTIONS
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id, i))
        .collect();
    let feature_section: std::collections::HashMap<&str, &str> = LANGUAGE_SURFACE
        .iter()
        .map(|c| (c.feature, c.section))
        .collect();
    let mut wrong: Vec<String> = Vec::new();

    for cp_id in CHECKPOINT_SECTIONS {
        let Some(&cp_pos) = position.get(cp_id) else {
            wrong.push(format!("checkpoint `{cp_id}` does not exist"));
            continue;
        };
        let cp = &SECTIONS[cp_pos];
        if !cp.examples.is_empty() {
            wrong.push(format!(
                "[{cp_id}] checkpoints carry exercises and recap, never examples (nothing new is \
                 taught in review)"
            ));
        }
        if let Some(next) = SECTIONS.get(cp_pos + 1) {
            if next.part == cp.part {
                wrong.push(format!(
                    "[{cp_id}] must be the LAST section of {:?} — the part closes on retrieval",
                    cp.part
                ));
            }
        }
        if cp.exercises.len() < 3 {
            wrong.push(format!(
                "[{cp_id}] {} exercises — a checkpoint carries ≥3",
                cp.exercises.len()
            ));
        }
        let part_start = SECTIONS
            .iter()
            .position(|s| s.part == cp.part)
            .expect("checkpoint's own part exists");
        for e in cp.exercises.iter() {
            let mut drawn: std::collections::HashSet<&str> = std::collections::HashSet::new();
            let mut reaches_back = false;
            for c in e.concepts {
                if let Some(&sec) = feature_section.get(c) {
                    drawn.insert(sec);
                    if position.get(sec).is_some_and(|&p| p < part_start) {
                        reaches_back = true;
                    }
                }
            }
            if drawn.len() < 2 {
                wrong.push(format!(
                    "[{cp_id}] `{}` blends {} section(s) — checkpoint exercises interleave ≥2 \
                     (Rohrer: blocked practice flatters, interleaved practice teaches)",
                    e.id,
                    drawn.len()
                ));
            }
            for sec in &drawn {
                if position.get(sec).is_some_and(|&p| p >= cp_pos) {
                    wrong.push(format!(
                        "[{cp_id}] `{}` draws on [{sec}] which is not earlier than the checkpoint",
                        e.id
                    ));
                }
            }
            let _ = reaches_back;
        }
        // Parts ≥ II: at least one exercise reaches back into an earlier part.
        if part_start > 0 {
            let any_reach = cp.exercises.iter().any(|e| {
                e.concepts.iter().any(|c| {
                    feature_section
                        .get(c)
                        .and_then(|sec| position.get(sec))
                        .is_some_and(|&p| p < part_start)
                })
            });
            if !any_reach {
                wrong.push(format!(
                    "[{cp_id}] no exercise reaches back to an earlier part — spacing needs distance"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\ncheckpoint violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn checkpoints_teach_nothing_new() {
    let mut wrong: Vec<String> = Vec::new();
    for c in LANGUAGE_SURFACE {
        if is_checkpoint(c.section) {
            wrong.push(format!(
                "{:?} claims checkpoint {:?} as its teaching home — checkpoints only review",
                c.feature, c.section
            ));
        }
    }
    assert!(wrong.is_empty(), "\n{}\n", wrong.join("\n"));
}

// ─── P8/P9: mental models + look-backs ───

#[test]
fn mental_models_are_single_and_registered() {
    let done: std::collections::HashSet<&str> = MENTAL_MODEL_DONE.iter().copied().collect();
    let required: std::collections::HashSet<&str> = MENTAL_MODEL_REQUIRED.iter().copied().collect();
    let mut wrong: Vec<String> = Vec::new();

    // Endgame form (W8): every REQUIRED section has its box — the ratchet closed.
    for r in MENTAL_MODEL_REQUIRED {
        if !done.contains(r) {
            wrong.push(format!(
                "[{r}] is REQUIRED but not DONE — the abstraction still lacks its mental model"
            ));
        }
    }
    for d in MENTAL_MODEL_DONE {
        if !required.contains(d) {
            wrong.push(format!(
                "[{d}] is in MENTAL_MODEL_DONE but not REQUIRED — an analogy where none is needed \
                 is decoration"
            ));
        }
    }
    for s in SECTIONS.iter() {
        let count = s.content.matches(MENTAL_MODEL_MARKER).count();
        if count > 1 {
            wrong.push(format!(
                "[{}] {count} mental-model boxes — two analogies fight and the reader keeps neither",
                s.id
            ));
        }
        if count == 1 && !done.contains(s.id) {
            wrong.push(format!(
                "[{}] carries a mental-model box but is not registered in MENTAL_MODEL_DONE — \
                 register it (the ratchet is the record)",
                s.id
            ));
        }
        if count == 0 && done.contains(s.id) {
            wrong.push(format!("[{}] is registered DONE but carries no box", s.id));
        }
        for line in s.content.lines() {
            if line.contains(MENTAL_MODEL_MARKER) && !line.trim_start().starts_with("> ") {
                wrong.push(format!(
                    "[{}] mental-model marker must live in a `> ` blockquote line",
                    s.id
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nmental-model violation(s):\n{}\n",
        wrong.join("\n")
    );
}

#[test]
fn look_back_blocks_are_registered() {
    let done: std::collections::HashSet<&str> = LOOK_BACK_DONE.iter().copied().collect();
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        let count = s.content.matches(LOOK_BACK_MARKER).count();
        if count > 0 && !done.contains(s.id) {
            wrong.push(format!(
                "[{}] carries a Look back block but is not registered in LOOK_BACK_DONE",
                s.id
            ));
        }
        if count == 0 && done.contains(s.id) {
            wrong.push(format!(
                "[{}] is registered DONE but carries no Look back block",
                s.id
            ));
        }
        for line in s.content.lines() {
            if line.contains(LOOK_BACK_MARKER) && !line.trim_start().starts_with("> ") {
                wrong.push(format!(
                    "[{}] Look back marker must live in a `> ` blockquote line",
                    s.id
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\nlook-back violation(s):\n{}\n",
        wrong.join("\n")
    );
}

// ─── tone ───

#[test]
fn tone_never_patronizes() {
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        for (where_, text) in teaching_text(s) {
            for w in BANNED_TONE_WORDS {
                if contains_word(&text, w) {
                    wrong.push(format!(
                        "[{}] {where_} contains {w:?} — if it were {w}, the reader wouldn't be here",
                        s.id
                    ));
                }
            }
            for p in BANNED_TONE_PHRASES {
                if text.to_lowercase().contains(p) {
                    wrong.push(format!("[{}] {where_} contains {p:?}", s.id));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "\ntone violation(s):\n{}\n",
        wrong.join("\n")
    );
}

// ─── misconception/recap id hygiene against DOM id spaces ───

#[test]
fn misconception_ids_do_not_collide() {
    let reserved: std::collections::HashSet<&str> = SECTIONS
        .iter()
        .flat_map(|s| {
            std::iter::once(s.id)
                .chain(s.examples.iter().map(|e| e.id))
                .chain(s.exercises.iter().map(|e| e.id))
                .chain(s.recap.iter().map(|r| r.id))
        })
        .collect();
    let mut wrong: Vec<String> = Vec::new();
    for s in SECTIONS.iter() {
        for m in s.misconceptions {
            if reserved.contains(m.id) {
                wrong.push(format!(
                    "misconception id `{}` collides with another id space",
                    m.id
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "\n{}\n", wrong.join("\n"));
}

// ─── P12: SM-2 over guide cards ───

#[test]
fn guide_srs_keys_are_namespaced() {
    assert!(recall_key("one-based").starts_with(GUIDE_RECALL_PREFIX));
    assert!(exercise_key("ex-greet-logos").starts_with(GUIDE_EX_PREFIX));
    assert_ne!(GUIDE_RECALL_PREFIX, GUIDE_EX_PREFIX);
    assert!(
        GUIDE_RECALL_PREFIX.starts_with("guide/") && GUIDE_EX_PREFIX.starts_with("guide/"),
        "guide card keys live under the guide/ namespace so Learn and Guide decks never collide"
    );
}

#[test]
fn recall_marks_schedule_reviews() {
    let mut p = UserProgress::new();
    // A shaky self-mark (quality 3) schedules a short interval; the card is
    // due again the next day, not today.
    record_recall_mark(&mut p, "rq-test-alpha", 3, "2026-07-13");
    let entry = p
        .exercises
        .get(&recall_key("rq-test-alpha"))
        .expect("the self-mark seeds an SRS entry under the namespaced key");
    assert_eq!(entry.srs.repetitions, 1);
    assert_eq!(
        entry.srs.next_review.as_deref(),
        Some("2026-07-14"),
        "first correct recall schedules a 1-day interval (SM-2)"
    );

    // A failed mark resets repetitions and keeps the card in rotation.
    record_recall_mark(&mut p, "rq-test-alpha", 1, "2026-07-14");
    let entry = p.exercises.get(&recall_key("rq-test-alpha")).unwrap();
    assert_eq!(
        entry.srs.repetitions, 0,
        "a failed recall resets the SM-2 chain"
    );
}

#[test]
fn exercise_results_schedule_reviews() {
    let mut p = UserProgress::new();
    record_exercise_result(&mut p, "ex-greet-logos", 5, "2026-07-13");
    let entry = p
        .exercises
        .get(&exercise_key("ex-greet-logos"))
        .expect("a graded pass seeds an SRS entry under the namespaced key");
    assert!(entry.srs.next_review.is_some());
    assert_eq!(entry.correct_count, 1);
}

#[test]
fn due_guide_cards_selects_only_seeded_due_cards() {
    let mut p = UserProgress::new();
    // Nothing seeded → nothing due (the guide deck never nags about cards the
    // reader has not touched).
    assert!(due_guide_cards(&p, "2026-07-13").is_empty());

    record_exercise_result(&mut p, "ex-greet-logos", 5, "2026-07-13");
    let due_now = due_guide_cards(&p, "2026-07-20");
    assert!(
        due_now.iter().any(|c| match c {
            GuideCard::Exercise {
                exercise_id,
                section_id,
                ..
            } => exercise_id == "ex-greet-logos" && section_id == "getting-started",
            _ => false,
        }),
        "a seeded exercise past its interval is due, and the card knows its section: {due_now:?}"
    );
    let due_before = due_guide_cards(&p, "2026-07-13");
    assert!(
        !due_before.iter().any(|c| matches!(
            c,
            GuideCard::Exercise { exercise_id, .. } if exercise_id == "ex-greet-logos"
        )),
        "a card inside its interval is not due"
    );
}

#[test]
fn guide_srs_entries_survive_serde() {
    let mut p = UserProgress::new();
    record_recall_mark(&mut p, "rq-test-beta", 4, "2026-07-13");
    record_exercise_result(&mut p, "ex-greet-logos", 5, "2026-07-13");
    let json = serde_json::to_string(&p).expect("serialize");
    let back: UserProgress = serde_json::from_str(&json).expect("deserialize");
    assert!(back.exercises.contains_key(&recall_key("rq-test-beta")));
    assert!(back.exercises.contains_key(&exercise_key("ex-greet-logos")));
}

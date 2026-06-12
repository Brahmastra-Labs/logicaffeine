#!/usr/bin/env python3
"""place-word.py — a flowchart that decides WHERE a word belongs in the LOGOS lexicon AND
what else it needs WIRED.

Adding a word is rarely one JSON line. This tool:
  • routes the word to the right section of `crates/logicaffeine_language/assets/lexicon.json`,
  • scans EVERY section first and flags homonyms / existing occurrences (a new POS for an
    existing word changes parsing — the lexer emits an `Ambiguous` token),
  • prints the exact JSON to paste, validated against the lexicon's real enums
    (read live from `crates/logicaffeine_lexicon/src/types.rs`, `src/ast/logic.rs`,
    `src/token.rs`, with hardcoded fallbacks),
  • and ADVISES the companion edits a word of that kind usually needs (the "second spots":
    build.rs singularizer, token.rs TokenType, disambiguation_not_verbs, axioms/ontology…).

Print-only by design (the harness proposes, you apply). After pasting, run `cargo build`.

Usage:
    ./scripts/place-word.py                 # interactive
    ./scripts/place-word.py --word genus    # pre-seed the word
    ./scripts/place-word.py --lexicon PATH  # different lexicon.json
"""
from __future__ import annotations
import argparse
import json
import os
import re
import sys
from collections import Counter

# ── Hardcoded enum fallbacks (verified against the crate; used only if parsing fails) ────────
_FALLBACK = {
    "VerbClass": ["State", "Activity", "Accomplishment", "Achievement", "Semelfactive"],
    "Sort": ["Entity", "Physical", "Animate", "Human", "Plant", "Place", "Time", "Abstract",
             "Information", "Event", "Celestial", "Value", "Signal", "Group"],
    "Feature": ["Transitive", "Intransitive", "Ditransitive", "SubjectControl", "ObjectControl",
                "Raising", "Opaque", "Factive", "Performative", "Collective", "Mixed",
                "Distributive", "Weather", "Unaccusative", "IntensionalPredicate", "Count",
                "Mass", "Proper", "Masculine", "Feminine", "Neuter", "Animate", "Inanimate",
                "Intersective", "NonIntersective", "Subsective", "Gradable", "EventModifier"],
    "Definiteness": ["Definite", "Indefinite", "Proximal", "Distal"],
    "Time": ["Past", "Present", "Future", "None"],
    "Number": ["Singular", "Plural"],
    "Gender": ["Male", "Female", "Neuter", "Unknown"],
    "Case": ["Subject", "Object", "Possessive"],
    "Dimension": ["Length", "Time", "Weight", "Temperature", "Cardinality"],
    "PresupKind": ["Stop", "Start", "Regret", "Continue", "Realize", "Know"],
}
# Curated POS→feature subsets (human knowledge; the crate has ONE flat Feature enum).
NOUN_FEATURE_NAMES = ["Count", "Mass", "Proper", "Masculine", "Feminine", "Neuter",
                      "Animate", "Inanimate"]
VERB_FEATURE_NAMES = ["Transitive", "Intransitive", "Ditransitive", "SubjectControl",
                      "ObjectControl", "Raising", "Opaque", "Factive", "Performative",
                      "Collective", "Mixed", "Distributive", "Weather", "Unaccusative",
                      "IntensionalPredicate"]
ADJ_FEATURE_NAMES = ["Intersective", "NonIntersective", "Subsective", "Gradable", "EventModifier"]
MWE_POS = ["Noun", "Verb", "Preposition", "Conjunction", "Quantifier"]
MORPH_BASE_POS = ["Noun", "Verb"]
MORPH_RELATION = ["Practitioner", "Agent", "Patient"]

# ── Disambiguation help (shown on "?" at the relevant prompt) ─────────────────────────────────
HELP_VERBCLASS = (
    "Vendler class — how the action sits in time:\n"
    "  State          ongoing property, no change ........ know, love, seem, exist, own\n"
    "  Activity       ongoing process, no endpoint ....... run, swim, walk, talk, push\n"
    "  Accomplishment process WITH a completion .......... build, write, draw, paint\n"
    "  Achievement    instantaneous change of state ...... find, win, die, arrive, reach\n"
    "  Semelfactive   single punctual act ................ knock, cough, blink, tap\n"
    "  Test: takes time to finish? Accomplishment. Sudden? Achievement. No endpoint? Activity.")
HELP_SORT = (
    "Sort = semantic type. Pick the MOST SPECIFIC that ALWAYS applies (skip if unsure).\n"
    "  Hierarchy: Human < Animate < Physical < Entity ; Plant < Animate.\n"
    "  Human(person) Animate(dog) Plant(fern) Physical(rock,car) Place(city) Time(hour)\n"
    "  Abstract(idea) Information(message) Event(meeting) Celestial(star) Value(price)\n"
    "  Signal(clock wire) Group(team) Entity(anything).")
HELP_ADJ = (
    "Adjective features (an adjective may have several — pick all that fit):\n"
    "  Intersective    'red car' IS red AND a car ........ red, wooden, female, dead\n"
    "  Subsective      depends on the noun ('tall for a') . tall, big, good, skilled\n"
    "  NonIntersective 'fake gun' is NOT a gun ............ fake, former, alleged, would-be\n"
    "  Gradable        takes very / more / -est ........... tall, happy, fast, bright\n"
    "  EventModifier   modifies an action, not a thing .... careful, quick, slow")
HELP_ROLE = (
    "Pick by what the word DOES in the sentence:\n"
    "  names a THING ................ Common noun  (a unique named individual -> Proper name)\n"
    "  names an ACTION/state ........ Verb\n"
    "  DESCRIBES a thing ............ Adjective  (test: 'very X' or '-er'? yes -> adjective;\n"
    "                                 a fixed noun-modifier like 'stone wall' -> Common noun)\n"
    "  relates a noun ('in','with') . Preposition (stands alone); if it ONLY pairs with a verb\n"
    "                                 to change its meaning ('give UP') -> Phrasal-verb particle\n"
    "  modifies a verb/clause:\n"
    "     manner/degree (quickly) ... Plain adverb\n"
    "     scope (almost, allegedly) . Scope-bearing adverb\n"
    "     time (yesterday, now) ..... Temporal adverb\n"
    "     -ly but describes a noun .. -ly word that is NOT an adverb (friendly)\n"
    "  logic word (every/and/if/not)  Quantifier / logical keyword\n"
    "  focus(only/even), modal(be able to), contraction(don't), block header -> Special construct\n"
    "  A word can be several POS at once — the tool flags the homonym and how to wire it.")

ENUM: dict[str, list[str]] = {}  # filled by build_enum_table() at startup

# Sections that are nested one level under a top-level key.
NESTED_SECTIONS = ["axioms.nouns", "axioms.adjectives", "axioms.verbs",
                   "ontology.predicate_sorts", "morphology.needs_e_ing",
                   "morphology.needs_e_ed", "morphology.stemming_exceptions"]

ROLES = [
    ("Common noun (dog, genus, idea)", "nouns", "build_common_noun"),
    ("Proper name, single word (Socrates)", "nouns", "build_proper_name"),
    ("Verb (run, give, seem)", "verbs", "build_verb"),
    ("Adjective (red, tall, fake)", "adjectives", "build_adjective"),
    ("Preposition (from, with)", "prepositions", "build_string_member"),
    ("Plain adverb (quickly)", "adverbs", "build_string_member"),
    ("Scope-bearing adverb (almost, allegedly)", "scopal_adverbs", "build_string_member"),
    ("Temporal adverb (yesterday, now)", "temporal_adverbs", "build_string_member"),
    ("-ly word that is NOT an adverb (friendly)", "not_adverbs", "build_string_member"),
    ("Phrasal-verb particle (up, off)", "particles", "build_string_member"),
    ("Phrasal verb w/ idiomatic meaning (give up)", "phrasal_verbs", "build_phrasal_verb"),
    ("Fixed multi-word expression (fire engine)", "multi_word_expressions", "build_mwe"),
    ("Unit of measurement (inch)", "units", "build_unit"),
    ("Spelled-out number (seven)", "number_words", "build_number_word"),
    ("Pronoun", "pronouns", "build_pronoun"),
    ("Article / determiner (the, a, this)", "articles", "build_article"),
    ("Tense auxiliary (will, did)", "auxiliaries", "build_auxiliary"),
    ("Quantifier / logical keyword (every, and, if)", "keywords", "build_keyword"),
    ("Presupposition-trigger verb (stop, regret)", "presupposition_triggers", "build_presup"),
    ("Noun/adj homonym mis-read as a verb (bit, red)", "disambiguation_not_verbs", "build_string_member"),
    ("Productive derivational suffix (-er -> Agent)", "morphological_rules", "build_morph_rule"),
    ("Meaning postulate / axiom (bachelor => unmarried)", "axioms", "build_axiom"),
    ("Part-whole or predicate-sort fact", "ontology", "build_ontology"),
    ("Inflection irregularity (silent-e, stemming)", "morphology", "build_morphology"),
    ("Special construct (focus particle, modal, contraction, block header…)", "__special__", "build_special"),
]

# ── Enum loading (source of truth = the crate; fall back to hardcoded) ───────────────────────

def load_enum_variants(path, name):
    try:
        text = open(path, encoding="utf-8").read()
    except OSError:
        return None
    m = re.search(r"pub enum " + name + r"\s*\{(.*?)\n\}", text, re.S)
    if not m:
        return None
    out = []
    for line in m.group(1).splitlines():
        s = line.strip()
        if not s or s.startswith("//") or s.startswith("#"):
            continue
        m2 = re.match(r"([A-Z][A-Za-z0-9]*)", s)
        if m2:
            out.append(m2.group(1))
    return out or None

def build_enum_table(root):
    types = os.path.join(root, "crates/logicaffeine_lexicon/src/types.rs")
    logic = os.path.join(root, "crates/logicaffeine_language/src/ast/logic.rs")
    token = os.path.join(root, "crates/logicaffeine_language/src/token.rs")
    spec = {"VerbClass": types, "Sort": types, "Number": types, "Gender": types, "Case": types,
            "Definiteness": types, "Time": types, "Feature": types, "Dimension": logic,
            "PresupKind": token}
    table, drift = {}, []
    for name, path in spec.items():
        v = load_enum_variants(path, name)
        if v:
            table[name] = v
        else:
            table[name] = _FALLBACK[name]
            drift.append(name)
    # Drift detector: curated feature subsets must stay within the real Feature enum.
    full = set(table["Feature"])
    for label, subset in (("NOUN", NOUN_FEATURE_NAMES), ("VERB", VERB_FEATURE_NAMES),
                          ("ADJ", ADJ_FEATURE_NAMES)):
        missing = [f for f in subset if f not in full]
        if missing:
            drift.append(f"{label}_FEATURES no longer in Feature enum: {missing}")
    return table, drift

def feats(subset):
    """Curated subset intersected with the live Feature enum (drops any drifted names)."""
    full = set(ENUM["Feature"])
    return [f for f in subset if f in full]

# ── Prompt helpers ──────────────────────────────────────────────────────────────────────────

def _read(prompt):
    try:
        return input(prompt)
    except EOFError:
        print()
        sys.exit(0)

def ask_text(prompt, default=None):
    suffix = f" [{default}]" if default is not None else ""
    while True:
        v = _read(f"{prompt}{suffix}: ").strip()
        if v:
            return v
        if default is not None:
            return default

def ask_int(prompt):
    while True:
        try:
            return int(_read(f"{prompt}: ").strip())
        except ValueError:
            print("  enter an integer.")

def ask_yn(prompt, default=False):
    v = _read(f"{prompt} ({'Y/n' if default else 'y/N'}): ").strip().lower()
    return default if not v else v.startswith("y")

def choose(prompt, options, allow_skip=False, default=None, help=None):
    print(f"\n{prompt}")
    for i, opt in enumerate(options, 1):
        print(f"  {i:2}. {opt}{'  (default, press Enter)' if opt == default else ''}")
    if allow_skip:
        print("   0. (skip / none)")
    if help:
        print("   ?  (explain the options)")
    while True:
        v = _read("  > ").strip()
        if v == "?" and help:
            print("\n" + help + "\n")
            continue
        if not v and default is not None:
            return default
        if allow_skip and v in ("0", ""):
            return None
        if v.isdigit() and 1 <= int(v) <= len(options):
            return options[int(v) - 1]
        print("  pick a number from the list" + (" (or ? to explain)" if help else "") + ".")

def choose_many(prompt, options, help=None):
    print(f"\n{prompt}  (comma-separated numbers, blank for none{', ? to explain' if help else ''})")
    for i, opt in enumerate(options, 1):
        print(f"  {i:2}. {opt}")
    while True:
        v = _read("  > ").strip()
        if v == "?" and help:
            print("\n" + help + "\n")
            continue
        if not v:
            return []
        try:
            idx = [int(x) for x in v.replace(" ", "").split(",")]
            if all(1 <= i <= len(options) for i in idx):
                return list(dict.fromkeys(options[i - 1] for i in idx))   # de-dup, keep order
        except ValueError:
            pass
        print("  comma-separated numbers from the list, e.g. 1,3" + (" (or ? to explain)" if help else ""))

def csv_text(prompt):
    v = ask_text(prompt, default="")
    return [x.strip() for x in v.split(",") if x.strip()]

def cap(word):
    return word[:1].upper() + word[1:] if word else word

# ── Builders: return ("section", entry, note) ────────────────────────────────────────────────

def build_common_noun(word, lex):
    entry, note = {"lemma": cap(word)}, None
    low = word.lower()
    if low.endswith("s") and not low.endswith("ss"):
        print(f"\n  ! '{word}' is a singular noun ending in -s; the singularizer would mangle it.")
        if ask_yn("    Plural is the SAME word (invariant, like 'species')?", default=False):
            entry["forms"] = {"plural": low}
        else:
            distinct = ask_text("    True plural (e.g. genera), or blank if unsure", default="")
            if distinct:
                entry["forms"] = {"plural": distinct.lower()}
                note = ("A singular -s noun with a DISTINCT plural is the one case the single "
                        "forms.plural field can't fully capture — if the singular mis-renders, "
                        "that is a real escalation signal (fix the machinery, not the word).")
    elif ask_yn("Irregular plural? (mice, geese)", default=False):
        entry["forms"] = {"plural": ask_text("  plural form").lower()}
    sug = suggest_noun_sort(lex, word)
    sort = choose("Sort? (semantic type; skip to omit)", ENUM["Sort"], allow_skip=True,
                  default=sug, help=HELP_SORT)
    if sort:
        entry["sort"] = sort
    f = choose_many("Noun features?", feats(NOUN_FEATURE_NAMES))
    if f:
        entry["features"] = f
    if ask_yn("Agentive/derived noun? (e.g. hunter <- hunt)", default=False):
        entry["derivation"] = {"root": cap(ask_text("  root word")),
                               "pos": choose("  root POS", ["Verb", "Noun"]),
                               "relation": choose("  relation", MORPH_RELATION)}
    return "nouns", entry, note

def build_proper_name(word, lex):
    entry = {"lemma": cap(word), "features": ["Proper"]}
    g = choose("Gender feature? (skip to omit)", ["Masculine", "Feminine", "Neuter"], allow_skip=True)
    if g:
        entry["features"].append(g)
    return "nouns", entry, None

def build_verb(word, lex):
    entry = {"lemma": cap(word), "class": choose("Vendler class?", ENUM["VerbClass"], help=HELP_VERBCLASS)}
    if ask_yn("Irregular forms? (enter past/participle/etc.)", default=False):
        forms = {}
        for k in ("present3s", "past", "participle", "gerund"):
            v = ask_text(f"  {k} (blank to skip)", default="")
            if v:
                forms[k] = v
        if forms:
            entry["forms"] = forms
    elif ask_yn("Mark as regular? (predictable -s/-ed/-ing)", default=True):
        entry["regular"] = True
    f = choose_many("Verb features? (transitivity/control/semantic)", feats(VERB_FEATURE_NAMES))
    if f:
        entry["features"] = f
    syn = csv_text("Synonyms? (comma-separated, blank to skip)")
    if syn:
        entry["synonyms"] = syn
    ant = csv_text("Antonyms? (comma-separated, blank to skip)")
    if ant:
        entry["antonyms"] = ant
    return "verbs", entry, None

def build_adjective(word, lex):
    entry = {"lemma": cap(word), "regular": ask_yn("Regular comparative? (-er/-est)", default=True)}
    f = choose_many("Adjective features?", feats(ADJ_FEATURE_NAMES), help=HELP_ADJ)
    if f:
        entry["features"] = f
    return "adjectives", entry, None

def build_string_member(word, lex):
    return None, word.lower(), None

def build_phrasal_verb(word, lex):
    verb = ask_text("Verb part (e.g. give)", default=word).lower()
    particle = ask_text("Particle (e.g. up)").lower()
    key = f"{verb}_{particle}"
    entry = {"lemma": cap(ask_text("Meaning lemma (e.g. Surrender)")),
             "class": choose("Vendler class?", ENUM["VerbClass"])}
    return "phrasal_verbs", {key: entry}, f'key = "{key}"'

def build_mwe(word, lex):
    pat = ask_text("Pattern words, space-separated (e.g. fire engine)", default=word)
    entry = {"pattern": [w.lower() for w in pat.split()],
             "lemma": ask_text("Lemma in CamelCase (e.g. FireEngine)"),
             "pos": choose("POS?", MWE_POS)}
    if entry["pos"] == "Verb":
        entry["class"] = choose("Vendler class?", ENUM["VerbClass"])
    f = choose_many("Features? (e.g. Proper)", feats(NOUN_FEATURE_NAMES + VERB_FEATURE_NAMES))
    if f:
        entry["features"] = f
    return "multi_word_expressions", entry, None

def build_unit(word, lex):
    return "units", {word.lower(): choose("Dimension?", ENUM["Dimension"])}, None

def build_number_word(word, lex):
    return "number_words", {word.lower(): ask_int("Integer value")}, None

def build_pronoun(word, lex):
    return "pronouns", {"word": word.lower(),
                        "gender": choose("Gender?", ENUM["Gender"]),
                        "number": choose("Number?", ENUM["Number"]),
                        "case": choose("Case?", ENUM["Case"])}, None

def build_article(word, lex):
    return "articles", {word.lower(): choose("Definiteness?", ENUM["Definiteness"])}, None

def build_auxiliary(word, lex):
    return "auxiliaries", {word.lower(): choose("Time?", ENUM["Time"])}, None

def build_keyword(word, lex):
    return "keywords", {word.lower(): ask_text("TokenType name (must exist in src/token.rs)")}, None

def build_presup(word, lex):
    return "presupposition_triggers", {word.lower(): choose("PresupKind?", ENUM["PresupKind"])}, None

def build_morph_rule(word, lex):
    return "morphological_rules", {"suffix": ask_text("Suffix (e.g. er)", default=word).lower(),
                                   "base_pos": choose("base_pos?", MORPH_BASE_POS),
                                   "relation": choose("relation?", MORPH_RELATION)}, None

def build_axiom(word, lex):
    kind = choose("Axiom over a…", ["noun", "adjective", "verb"])
    if kind == "noun":
        body = {}
        ent, hyp = csv_text("Entails predicates (e.g. Unmarried, Male)"), csv_text("Hypernyms (e.g. Animal, Mammal)")
        if ent:
            body["entails"] = ent
        if hyp:
            body["hypernyms"] = hyp
        return "axioms.nouns", {word.lower(): body}, None
    if kind == "adjective":
        return "axioms.adjectives", {word.lower(): {"type": "Privative"}}, None
    body = {"entails": cap(ask_text("Entailed verb lemma (e.g. Kill)"))}
    manner = csv_text("Manner constraints (e.g. Intentional)")
    if manner:
        body["manner"] = manner
    return "axioms.verbs", {word.lower(): body}, None

def build_ontology(word, lex):
    if choose("Ontology fact…", ["part-whole", "predicate sort"]) == "part-whole":
        return "ontology.part_whole", {"whole": cap(ask_text("Whole (e.g. Car)", default=word)),
                                       "parts": [cap(p) for p in csv_text("Parts (e.g. Engine, Wheel)")]}, None
    return "ontology.predicate_sorts", {word.lower(): choose("Required subject Sort?", ENUM["Sort"])}, None

def build_morphology(word, lex):
    which = choose("Which list?", ["needs_e_ing", "needs_e_ed", "stemming_exceptions"])
    return f"morphology.{which}", ask_text("Stem / word to add", default=word).lower(), None

# ── Lexicon inspection ───────────────────────────────────────────────────────────────────────

def get_section(lex, dotted):
    node = lex
    for p in dotted.split("."):
        if not isinstance(node, dict):
            return None
        node = node.get(p)
    return node

def section_members(lex, dotted):
    """Normalized identifiers already present in a (possibly nested) section."""
    node, keys = get_section(lex, dotted), set()
    if isinstance(node, dict):
        keys = {k.lower() for k in node}
    elif isinstance(node, list):
        for e in node:
            if isinstance(e, str):
                keys.add(e.lower())
            elif isinstance(e, dict):
                if "pattern" in e:
                    keys.add(" ".join(w.lower() for w in e["pattern"]))
                for k in ("lemma", "whole", "word", "suffix"):
                    if k in e:
                        keys.add(e[k].lower())
    return keys

def find_all_occurrences(lex, word):
    w = word.lower()
    hits = []
    for sec in list(lex.keys()) + NESTED_SECTIONS:
        if sec == "noun_patterns":          # a test fixture, not a real occurrence
            continue
        if sec in lex and not isinstance(lex[sec], (list, dict)):
            continue
        if w in section_members(lex, sec):
            hits.append(sec)
    return hits

def entry_name(entry, word):
    if isinstance(entry, str):
        return entry.lower()
    if isinstance(entry, dict):
        if "pattern" in entry:
            return " ".join(entry["pattern"])
        for k in ("whole", "lemma", "word", "suffix"):
            if k in entry:
                return entry[k].lower()
        return next(iter(entry)).lower()
    return word.lower()

def siblings(lex, section, like=None, n=3):
    node = get_section(lex, section)
    if not isinstance(node, list):
        return []
    out = []
    for e in node:
        if isinstance(e, dict) and (not like or all(e.get(k) == v for k, v in like.items())):
            out.append(e)
        if len(out) >= n:
            break
    return out

def suggest_noun_sort(lex, word):
    w = word.lower()
    cohort = [n.get("sort") for n in lex.get("nouns", [])
              if isinstance(n, dict) and n.get("sort") and n.get("lemma")
              and n["lemma"].lower()[-3:] == w[-3:]]
    return Counter(cohort).most_common(1)[0][0] if cohort else None

# ── Companion-edit advisor (the "second spots") ──────────────────────────────────────────────

def companion_edits(section, entry, word, lex, root):
    top = section.split(".")[0]
    notes = []

    others = [s for s in find_all_occurrences(lex, word) if s.split(".")[0] != top]
    if others:
        notes.append(
            f"HOMONYM: '{word}' already exists in [{', '.join(others)}]. A word that is a verb "
            "AND a noun/adjective makes the lexer emit an `Ambiguous` token (src/lexer.rs ~2414), "
            "changing the parse of existing sentences. For a single-POS reading, also wire "
            "`disambiguation_not_verbs` (src/lexer.rs ~2412). This is a behavior change — "
            "consider escalating rather than silently adding.")

    if top == "nouns" and isinstance(entry, dict):
        low = word.lower()
        if low.endswith("s") and not low.endswith("ss") and "forms" not in entry:
            notes.append(
                "-s SINGULAR: the singularizer (src/parser/mod.rs::singularize_noun ~1018) strips "
                "trailing -s. Set forms.plural — invariant nouns use plural==singular (like "
                "'species'); the mapping is generated in build.rs::generate_singularize ~922.")
        if "sort" in entry:
            notes.append(
                f"sort='{entry['sort']}': sorted nouns often carry meaning postulates — consider "
                "`axioms.nouns` (hypernyms/entails, e.g. dog->[Animal,Mammal]) and "
                "`ontology.part_whole` if the thing has parts.")
        deriv = entry.get("derivation")
        if deriv:
            root = deriv.get("root", "").lower()
            sec = "verbs" if deriv.get("pos") == "Verb" else "nouns"
            if root and root not in section_members(lex, sec):
                notes.append(
                    f"DERIVATION root '{deriv['root']}' is NOT yet in `{sec}` — the derivation "
                    "resolves but the base word won't be recognized. Add the root too.")

    if section == "phrasal_verbs" and isinstance(entry, dict):
        key = next(iter(entry))
        verb, _, particle = key.partition("_")
        miss = [(w, s) for w, s in ((verb, "verbs"), (particle, "particles"))
                if w and w not in section_members(lex, s)]
        if miss:
            notes.append("PHRASAL base parts missing: " + ", ".join(f"'{w}' not in `{s}`" for w, s in miss)
                         + " — add them first, else the phrasal won't resolve.")

    if top == "verbs":
        notes.append(
            "VERB companions to consider: `phrasal_verbs` (give_up, break_down); `axioms.verbs` if "
            "it entails another verb (murder->Kill, manner=[Intentional]); `presupposition_triggers` "
            "if it presupposes its complement (stop/regret/realize).")

    if top == "keywords" and isinstance(entry, dict):
        tok = next(iter(entry.values()))
        msg = ("KEYWORD needs TWO companion edits or it SILENTLY no-ops:\n"
               f"    1. src/token.rs — `TokenType::{tok}` must exist.\n"
               f'    2. build.rs generate_lookup_keyword (~674) — add: "{tok}" => '
               f'"crate::token::TokenType::{tok}",\n'
               "       Without that arm, build.rs's `_ => continue` drops the keyword with NO error.")
        tokfile = os.path.join(root, "crates/logicaffeine_language/src/token.rs")
        if os.path.exists(tokfile):
            present = re.search(r"\b" + re.escape(tok) + r"\b", open(tokfile, encoding="utf-8").read())
            msg += (f"\n    [live check] token.rs {'mentions' if present else 'does NOT mention'} "
                    f"'{tok}'" + ("." if present else " — you must add the variant first.")
                    )
        notes.append(msg)

    if section == "multi_word_expressions":
        notes.append("MWE: do NOT also add the component words as separate entries unless they're "
                     "used independently. Lemma is CamelCase; verb MWEs need a `class`.")
    return notes

def compact(obj):
    # Mirror lexicon.json's inline style (padded braces) WITHOUT touching string contents,
    # so a value like "foo{bar}" is never mutated.
    if isinstance(obj, dict):
        inner = ", ".join(f"{json.dumps(k, ensure_ascii=False)}: {compact(v)}" for k, v in obj.items())
        return "{ " + inner + " }" if inner else "{}"
    if isinstance(obj, list):
        return "[" + ", ".join(compact(v) for v in obj) + "]"
    return json.dumps(obj, ensure_ascii=False)

# ── Special constructs that are NOT pure lexicon.json (hardcoded in Rust) ─────────────────────

HARDCODED_SPOTS = """\
Some "word additions" are NOT lexicon.json entries — they are wired in Rust. If your word is
one of these, edit the spot below (a lexicon.json entry alone will NOT work):

  Focus particle (only, even, just) ......... src/lexer.rs ~2263  (+ FocusKind in token.rs)
  Multi-word quantifier (at least/most N,
    exactly N, each other) .................. src/lexer.rs ~1883-1919
  Identity / biconditional synonym
    (is equal to, is identical to, iff) ..... src/lexer.rs ~1956-1966
  Periphrastic modal (be able to, …) ........ src/parser/modal.rs ~115
  Contraction (don't, won't, mustn't) ....... src/lexer.rs ~996
  Block header (## Theorem:, ## Property:) .. src/lexer.rs ~1850  (+ BlockType in token.rs)
  Time literal (noon, midnight, am/pm) ...... src/lexer.rs ~1715-1800
  Hardcoded noun fallback
    (logic, time, book, house, user, …) ..... src/lexer.rs ~2477
  Keyword token mapping ..................... src/token.rs (TokenType) + build.rs
    generate_lookup_keyword ~674  (its `_ => continue` SILENTLY drops unmapped keywords)

These are higher blast-radius than data adds — treat as parser changes (human-supervised).
The `noun_patterns` lexicon section is a test fixture, not a real word-add path.
"""

def build_special(word, lex):
    print("\n" + HARDCODED_SPOTS)
    return "__special__", None, None

# ── Auto-apply: targeted text insertion that preserves the file's hand formatting ────────────

def match_bracket(text, i):
    depth, in_str, esc, j = 0, False, False, i
    while j < len(text):
        c = text[j]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c in "[{":
            depth += 1
        elif c in "]}":
            depth -= 1
            if depth == 0:
                return j
        j += 1
    return -1

def locate_container(text, dotted):
    start, end, open_idx, close_idx = 0, len(text), -1, -1
    for p in dotted.split("."):
        m = re.search(r'"' + re.escape(p) + r'"\s*:\s*([\[{])', text[start:end])
        if not m:
            return None
        open_idx = start + m.start(1)
        close_idx = match_bracket(text, open_idx)
        if close_idx < 0:
            return None
        start, end = open_idx + 1, close_idx
    return open_idx, close_idx

def _format_element(bracket, entry):
    if bracket == "{":
        k = next(iter(entry))
        v = entry[k]
        val = compact(v) if isinstance(v, dict) else json.dumps(v, ensure_ascii=False)
        return json.dumps(k, ensure_ascii=False) + ": " + val
    return json.dumps(entry, ensure_ascii=False) if isinstance(entry, str) else compact(entry)

def insert_entry(text, dotted, entry):
    loc = locate_container(text, dotted)
    if not loc:
        return None
    open_idx, close_idx = loc
    el = _format_element(text[open_idx], entry)

    def indent_of(pos):
        ls = text.rfind("\n", 0, pos) + 1
        line = text[ls:pos]
        return line[:len(line) - len(line.lstrip())]

    close_indent = indent_of(close_idx)
    j = close_idx - 1
    while j > open_idx and text[j] in " \t\r\n":
        j -= 1
    if j == open_idx:                                   # empty container
        units = {len(l) - len(l.lstrip(" ")) for l in text.splitlines() if l[:1] == " " and l.strip()}
        step = " " * (min(units) if units else 2)
        body = "\n" + close_indent + step + el + "\n" + close_indent
        return text[:open_idx + 1] + body + text[close_idx:]
    sep = "" if text[j] == "," else ","
    return text[:j + 1] + sep + "\n" + indent_of(j) + el + "\n" + close_indent + text[close_idx:]

def apply_entry(path, dotted, entry):
    """Insert, validate the result still parses, then write. Returns (ok, message)."""
    text = open(path, encoding="utf-8").read()
    new = insert_entry(text, dotted, entry)
    if new is None:
        return False, f"could not locate section `{dotted}`"
    try:
        json.loads(new)
    except json.JSONDecodeError as e:
        return False, f"insertion would break JSON ({e}) — not written"
    open(path, "w", encoding="utf-8").write(new)
    return True, f"inserted into `{dotted}`"

# ── Lexical grounding (OPTIONAL deps: inflect, lemminflect, nltk+WordNet) ─────────────────────
# Maps WordNet noun "supersenses" (lexicographer files) to our Sort enum. Sort is sense-
# dependent, so we surface candidates across senses rather than auto-pick one.
SUPERSENSE_TO_SORT = {
    "noun.animal": "Animate", "noun.person": "Human", "noun.plant": "Plant",
    "noun.location": "Place", "noun.time": "Time", "noun.group": "Group",
    "noun.communication": "Information", "noun.cognition": "Abstract",
    "noun.attribute": "Abstract", "noun.feeling": "Abstract", "noun.state": "Abstract",
    "noun.motive": "Abstract", "noun.relation": "Abstract", "noun.event": "Event",
    "noun.act": "Event", "noun.process": "Event", "noun.phenomenon": "Event",
    "noun.artifact": "Physical", "noun.object": "Physical", "noun.body": "Physical",
    "noun.food": "Physical", "noun.substance": "Physical", "noun.shape": "Physical",
    "noun.possession": "Value", "noun.quantity": "Value", "noun.Tops": "Entity",
}

def _transitivity_from_frames(frames, verb):
    tails = [f.lower().split(verb.lower(), 1)[-1].strip() for f in frames if verb.lower() in f.lower()]
    if any("somebody something" in t or t.startswith("something to") for t in tails):
        return "Ditransitive"
    if any(t.startswith(("something", "somebody", "to ")) for t in tails):
        return "Transitive"
    return "Intransitive" if frames else None

def lexical_evidence(word):
    """POS-organised, evidence-backed lexical data from optional libs (inflect/lemminflect/
    WordNet). Deterministic morphology is reliable; sort/relations are SUGGESTIONS to confirm.
    `gaps` lists distinctions WordNet draws that the LOGOS lexicon has no home for."""
    w = word.lower()
    ev = {"sources": [], "pos": [], "plural": None, "verb_forms": None, "transitivity": None,
          "frames": [], "sort_candidates": [], "hypernyms": [], "part_whole": [], "synonyms": [],
          "antonyms": [], "derivation": None, "entails": [], "causes": [], "relational_to": [],
          "attribute": [], "gaps": []}
    try:
        import inflect
        ev["plural"] = inflect.engine().plural_noun(w)
        ev["sources"].append("inflect")
    except Exception:
        pass
    try:
        from nltk.corpus import wordnet as wn
        ev["sources"].append("wordnet")
        nouns, verbs = wn.synsets(w, pos=wn.NOUN), wn.synsets(w, pos=wn.VERB)
        adjs = wn.synsets(w, pos=wn.ADJ) + wn.synsets(w, pos="s")
        ev["pos"] = [p for p, l in (("noun", nouns), ("verb", verbs), ("adjective", adjs)) if l]
        if len(ev["pos"]) > 1:
            ev["gaps"].append(f"WordNet lists this word as {ev['pos']} — confirm the POS in context "
                              "(LOGOS makes a verb+noun/adj word an Ambiguous token).")
        if nouns:
            seen = set()
            for s in nouns[:6]:
                srt = SUPERSENSE_TO_SORT.get(s.lexname())
                if srt and srt not in seen:
                    seen.add(srt)
                    ev["sort_candidates"].append({"sense": s.name(), "supersense": s.lexname(),
                                                  "sort": srt, "gloss": s.definition()[:70]})
            n0 = nouns[0]
            ev["hypernyms"] = [h.lemmas()[0].name() for h in n0.hypernyms()][:4]
            ev["part_whole"] = [m.lemmas()[0].name() for m in n0.part_meronyms()][:8]
            ev["synonyms"] = [l.name() for l in n0.lemmas() if l.name().lower() != w][:5]
            for l in n0.lemmas():
                ev["antonyms"] += [a.name() for a in l.antonyms()]
                if not ev["derivation"]:
                    for r in l.derivationally_related_forms():
                        if r.synset().pos() == "v":
                            ev["derivation"] = {"root": cap(r.name()), "pos": "Verb", "relation": "Agent"}
                            break
        if verbs:
            v0 = verbs[0]
            lem = v0.lemmas()[0]
            ev["frames"] = lem.frame_strings()[:4]
            ev["transitivity"] = _transitivity_from_frames(ev["frames"], w)
            ev["entails"] = [e.lemmas()[0].name() for e in v0.entailments()][:4]
            ev["causes"] = [c.lemmas()[0].name() for c in v0.causes()][:4]
            ev["antonyms"] += [a.name() for a in lem.antonyms()]
            try:
                import lemminflect
                vf = {t: lemminflect.getInflection(w, tag=t) for t in ("VBD", "VBN", "VBG", "VBZ")}
                if any(vf.values()):
                    ev["verb_forms"] = {k: vf[t][0] for k, t in
                                        (("past", "VBD"), ("participle", "VBN"),
                                         ("gerund", "VBG"), ("present3s", "VBZ")) if vf[t]}
                if "lemminflect" not in ev["sources"]:
                    ev["sources"].append("lemminflect")
            except Exception:
                pass
        if adjs:
            a0 = adjs[0]
            alem = a0.lemmas()[0]
            ev["relational_to"] = [p.name() for p in alem.pertainyms()]
            ev["attribute"] = [a.lemmas()[0].name() for a in a0.attributes()]
            ev["antonyms"] += [a.name() for a in alem.antonyms()]
            if ev["relational_to"]:
                ev["gaps"].append(
                    f"RELATIONAL adjective — pertains to noun {ev['relational_to']} (e.g. 'coastal'->"
                    "'coast'). LOGOS has no Relational/Pertainymic feature; it would be tagged "
                    "Subsective, which is wrong (relational adjectives don't predicate: '*the region "
                    "is coastal' is odd).")
            if ev["attribute"]:
                ev["gaps"].append(
                    f"gradable adjective with ATTRIBUTE dimension {ev['attribute']} (e.g. tall->stature). "
                    "LOGOS has Gradable but does not link an adjective to its scale/Dimension.")
        ev["antonyms"] = list(dict.fromkeys(ev["antonyms"]))[:5]
    except Exception:
        pass
    return ev

def print_evidence(word):
    ev = lexical_evidence(word)
    if not ev["sources"]:
        print("\n(no lexical libs available — install inflect/lemminflect/nltk for --enrich)")
        return ev
    print(f"\nEVIDENCE for '{word}'  (POS: {ev['pos'] or '?'}; sources: {', '.join(ev['sources'])}; "
          "confirm — do not trust blindly):")
    if ev["plural"]:
        print(f"  plural ................... {ev['plural']}")
    if ev["verb_forms"]:
        print(f"  verb forms .............. {ev['verb_forms']}")
    if ev["transitivity"]:
        print(f"  transitivity (frames) ... {ev['transitivity']}   e.g. {ev['frames'][:1]}")
    if ev["sort_candidates"]:
        print("  sort candidates (sense-dependent — PICK by the sentence):")
        for c in ev["sort_candidates"]:
            print(f"      {c['sort']:9} <- {c['supersense']:16} ({c['sense']}: {c['gloss']})")
    if ev["hypernyms"]:
        print(f"  hypernyms (-> axioms) ... {ev['hypernyms']}")
    if ev["part_whole"]:
        print(f"  parts (-> ontology) ..... {ev['part_whole']}")
    if ev["entails"]:
        print(f"  entails (-> axioms.verbs) {ev['entails']}")
    if ev["causes"]:
        print(f"  causes .................. {ev['causes']}")
    if ev["relational_to"]:
        print(f"  relational to (pertainym) {ev['relational_to']}")
    if ev["attribute"]:
        print(f"  attribute/scale ......... {ev['attribute']}")
    if ev["antonyms"]:
        print(f"  antonyms ................ {ev['antonyms']}")
    if ev["derivation"]:
        print(f"  derivation .............. {ev['derivation']}")
    if ev["synonyms"]:
        print(f"  synonyms ................ {ev['synonyms']}")
    if ev["gaps"]:
        print("  ⚠ SCHEMA GAPS (WordNet distinguishes; LOGOS does not):")
        for g in ev["gaps"]:
            print("      - " + g)
    return ev

# ── Non-interactive / agent mode: spec -> entry, validated, with JSON + exit codes ────────────

ROLE_SLUGS = ["noun", "proper-name", "verb", "adjective", "preposition", "adverb",
              "scopal-adverb", "temporal-adverb", "not-adverb", "particle", "phrasal-verb",
              "mwe", "unit", "number-word", "pronoun", "article", "auxiliary", "keyword",
              "presup", "disambiguation", "morph-rule", "axiom", "ontology", "morphology",
              "special"]
SLUG_SECTION = dict(zip(ROLE_SLUGS, [r[1] for r in ROLES]))
STRING_SLUGS = {"preposition", "adverb", "scopal-adverb", "temporal-adverb", "not-adverb",
                "particle", "disambiguation"}

# exit codes
EXIT_OK, EXIT_BLOCKED, EXIT_BAD_INPUT, EXIT_NOT_AUTO, EXIT_APPLY_FAILED = 0, 1, 2, 3, 5

class SpecError(Exception):
    def __init__(self, code, msg):
        self.code, self.msg = code, msg

def _venum(name, val):
    if val not in ENUM[name]:
        raise SpecError(EXIT_BAD_INPUT, f"{val!r} is not a valid {name}; options: {ENUM[name]}")
    return val

def _vfeatures(subset, vals):
    allowed = set(feats(subset))
    bad = [x for x in vals if x not in allowed]
    if bad:
        raise SpecError(EXIT_BAD_INPUT, f"invalid features {bad}; allowed: {sorted(allowed)}")
    return list(dict.fromkeys(vals))

def _req(f, key, slug):
    if key not in f:
        raise SpecError(EXIT_BAD_INPUT, f"role '{slug}' requires field '{key}'")
    return f[key]

def build_from_spec(slug, f, word):
    """Deterministically build (section, entry, note) from a fields dict. Raises SpecError."""
    if slug not in ROLE_SLUGS:
        raise SpecError(EXIT_BAD_INPUT, f"unknown role '{slug}'; valid: {ROLE_SLUGS}")
    if slug == "special":
        raise SpecError(EXIT_NOT_AUTO, "special constructs are wired in Rust, not lexicon data")
    if slug in STRING_SLUGS:
        return SLUG_SECTION[slug], word.lower(), None

    if slug == "noun":
        e = {"lemma": cap(word)}
        if f.get("plural"):
            e["forms"] = {"plural": str(f["plural"]).lower()}
        if f.get("sort"):
            e["sort"] = _venum("Sort", f["sort"])
        if f.get("features"):
            e["features"] = _vfeatures(NOUN_FEATURE_NAMES, f["features"])
        if f.get("derivation"):
            d = f["derivation"]
            pos = d.get("pos", "Verb")
            if pos not in ("Verb", "Noun"):
                raise SpecError(EXIT_BAD_INPUT, "derivation.pos must be Verb or Noun")
            if d.get("relation") not in MORPH_RELATION:
                raise SpecError(EXIT_BAD_INPUT, f"derivation.relation must be one of {MORPH_RELATION}")
            e["derivation"] = {"root": cap(_req(d, "root", "derivation")), "pos": pos,
                               "relation": d["relation"]}
        return "nouns", e, None
    if slug == "proper-name":
        e = {"lemma": cap(word), "features": ["Proper"]}
        if f.get("gender"):
            if f["gender"] not in ("Masculine", "Feminine", "Neuter"):
                raise SpecError(EXIT_BAD_INPUT, "gender must be Masculine/Feminine/Neuter")
            e["features"].append(f["gender"])
        return "nouns", e, None
    if slug == "verb":
        e = {"lemma": cap(word), "class": _venum("VerbClass", _req(f, "class", slug))}
        if f.get("forms"):
            forms = {k: v for k, v in f["forms"].items()
                     if k in ("present3s", "past", "participle", "gerund")}
            if forms:
                e["forms"] = forms
        elif f.get("regular"):
            e["regular"] = True
        if f.get("features"):
            e["features"] = _vfeatures(VERB_FEATURE_NAMES, f["features"])
        for k in ("synonyms", "antonyms"):
            if f.get(k):
                e[k] = list(f[k])
        return "verbs", e, None
    if slug == "adjective":
        e = {"lemma": cap(word), "regular": bool(f.get("regular", True))}
        if f.get("features"):
            e["features"] = _vfeatures(ADJ_FEATURE_NAMES, f["features"])
        return "adjectives", e, None
    if slug == "phrasal-verb":
        verb = str(f.get("verb", word)).lower()
        particle = str(_req(f, "particle", slug)).lower()
        key = f"{verb}_{particle}"
        return "phrasal_verbs", {key: {"lemma": cap(_req(f, "lemma", slug)),
                                       "class": _venum("VerbClass", _req(f, "class", slug))}}, None
    if slug == "mwe":
        pattern = f.get("pattern") or word.split()
        e = {"pattern": [w.lower() for w in pattern], "lemma": _req(f, "lemma", slug),
             "pos": (f["pos"] if f.get("pos") in MWE_POS
                     else (_ for _ in ()).throw(SpecError(EXIT_BAD_INPUT, f"pos must be one of {MWE_POS}")))}
        if e["pos"] == "Verb" and f.get("class"):
            e["class"] = _venum("VerbClass", f["class"])
        if f.get("features"):
            e["features"] = _vfeatures(NOUN_FEATURE_NAMES + VERB_FEATURE_NAMES, f["features"])
        return "multi_word_expressions", e, None
    if slug == "unit":
        return "units", {word.lower(): _venum("Dimension", _req(f, "dimension", slug))}, None
    if slug == "number-word":
        try:
            return "number_words", {word.lower(): int(_req(f, "value", slug))}, None
        except (TypeError, ValueError):
            raise SpecError(EXIT_BAD_INPUT, "value must be an integer")
    if slug == "pronoun":
        return "pronouns", {"word": word.lower(),
                            "gender": _venum("Gender", _req(f, "gender", slug)),
                            "number": _venum("Number", _req(f, "number", slug)),
                            "case": _venum("Case", _req(f, "case", slug))}, None
    if slug == "article":
        return "articles", {word.lower(): _venum("Definiteness", _req(f, "definiteness", slug))}, None
    if slug == "auxiliary":
        return "auxiliaries", {word.lower(): _venum("Time", _req(f, "time", slug))}, None
    if slug == "keyword":
        return "keywords", {word.lower(): str(_req(f, "token_type", slug))}, None
    if slug == "presup":
        return "presupposition_triggers", {word.lower(): _venum("PresupKind", _req(f, "presup", slug))}, None
    if slug == "morph-rule":
        bp, rel = f.get("base_pos"), f.get("relation")
        if bp not in MORPH_BASE_POS:
            raise SpecError(EXIT_BAD_INPUT, f"base_pos must be one of {MORPH_BASE_POS}")
        if rel not in MORPH_RELATION:
            raise SpecError(EXIT_BAD_INPUT, f"relation must be one of {MORPH_RELATION}")
        return "morphological_rules", {"suffix": str(f.get("suffix", word)).lower(),
                                       "base_pos": bp, "relation": rel}, None
    if slug == "axiom":
        kind = _req(f, "kind", slug)
        if kind == "noun":
            body = {}
            if f.get("entails"):
                body["entails"] = list(f["entails"])
            if f.get("hypernyms"):
                body["hypernyms"] = list(f["hypernyms"])
            return "axioms.nouns", {word.lower(): body}, None
        if kind == "adjective":
            return "axioms.adjectives", {word.lower(): {"type": f.get("type", "Privative")}}, None
        if kind == "verb":
            body = {"entails": cap(_req(f, "entails", slug))}
            if f.get("manner"):
                body["manner"] = list(f["manner"])
            return "axioms.verbs", {word.lower(): body}, None
        raise SpecError(EXIT_BAD_INPUT, "axiom kind must be noun/adjective/verb")
    if slug == "ontology":
        kind = _req(f, "kind", slug)
        if kind in ("part-whole", "part_whole"):
            return "ontology.part_whole", {"whole": cap(f.get("whole", word)),
                                           "parts": [cap(p) for p in f.get("parts", [])]}, None
        if kind in ("predicate-sort", "predicate_sort"):
            return "ontology.predicate_sorts", {word.lower(): _venum("Sort", _req(f, "sort", slug))}, None
        raise SpecError(EXIT_BAD_INPUT, "ontology kind must be part-whole or predicate-sort")
    if slug == "morphology":
        lst = _req(f, "list", slug)
        if lst not in ("needs_e_ing", "needs_e_ed", "stemming_exceptions"):
            raise SpecError(EXIT_BAD_INPUT, "list must be needs_e_ing/needs_e_ed/stemming_exceptions")
        return f"morphology.{lst}", str(f.get("stem", word)).lower(), None
    raise SpecError(EXIT_BAD_INPUT, f"role '{slug}' not supported in spec mode")

def run_noninteractive(args, lex, root):
    word = args.word
    fields = {}
    if args.spec:
        try:
            fields = json.loads(args.spec)
        except json.JSONDecodeError as e:
            return _emit(args, {"status": "error", "error": f"bad --spec JSON: {e}",
                                "exit_code": EXIT_BAD_INPUT})
        word = word or fields.get("word")
    slug = args.role or fields.get("role")
    if not word:
        return _emit(args, {"status": "error", "error": "no --word", "exit_code": EXIT_BAD_INPUT})
    if not slug:
        return _emit(args, {"status": "error", "error": "no --role (and none in --spec)",
                            "exit_code": EXIT_BAD_INPUT})
    try:
        section, entry, note = build_from_spec(slug, fields, word)
    except SpecError as e:
        return _emit(args, {"word": word, "role": slug, "status": "error", "error": e.msg,
                            "exit_code": e.code})

    # Tier-A grounding: fold the RELIABLE evidence into the entry; PROPOSE the rest as
    # confirmable companion entries (axioms/ontology) the caller can apply separately.
    proposals = []
    ev = lexical_evidence(word) if args.enrich else None
    if ev:
        if slug == "verb" and isinstance(entry, dict) and "features" not in fields and ev["transitivity"]:
            entry.setdefault("features", [])
            if ev["transitivity"] not in entry["features"]:
                entry["features"].append(ev["transitivity"])   # frames -> arity (deterministic)
        if ev["hypernyms"]:
            proposals.append({"section": "axioms.nouns",
                              "entry": {word.lower(): {"hypernyms": [cap(h) for h in ev["hypernyms"]]}},
                              "source": "wordnet.hypernyms", "confirm": True})
        if ev["part_whole"]:
            proposals.append({"section": "ontology.part_whole",
                              "entry": {"whole": cap(word), "parts": [cap(p) for p in ev["part_whole"]]},
                              "source": "wordnet.meronyms", "confirm": True})
        if ev["entails"]:
            proposals.append({"section": "axioms.verbs",
                              "entry": {word.lower(): {"entails": cap(ev["entails"][0])}},
                              "source": "wordnet.entailment", "confirm": True})

    top = section.split(".")[0]
    name = entry_name(entry, word)
    conflict = name in section_members(lex, section)
    homonym = [s for s in find_all_occurrences(lex, word) if s.split(".")[0] != top]
    advice = companion_edits(section, entry, word, lex, root)

    blockers = []
    if conflict:
        blockers.append("conflict")
    if homonym:
        blockers.append("homonym")
    if top == "keywords":
        blockers.append("keyword-needs-rust-wiring")

    status, applied, exit_code = "planned", False, EXIT_OK
    if conflict or homonym:
        exit_code = EXIT_BLOCKED
        status = "blocked"
    elif top == "keywords":
        exit_code = EXIT_NOT_AUTO
        status = "needs-wiring"
    if args.apply and not blockers:
        ok, msg = apply_entry(args.lexicon, section, entry)
        applied, status = ok, ("applied" if ok else "apply-failed")
        if not ok:
            exit_code = EXIT_APPLY_FAILED
    return _emit(args, {
        "word": word, "role": slug, "section": section, "entry": entry, "entry_name": name,
        "conflict": conflict, "homonym": homonym, "companion_warnings": advice,
        "applied": applied, "status": status, "note": note, "enum_drift": getattr(args, "_drift", []),
        "enrichment": ev, "proposals": proposals,
        "exit_code": exit_code,
        "next": (["cargo build", "re-triage"] if applied else
                 ["resolve: " + ", ".join(blockers)] if blockers else
                 ["paste entry into " + section, "cargo build"]),
    })

def _emit(args, result):
    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    else:
        print(f"[{result.get('status','?')}] {result.get('word','')} -> {result.get('section','-')}")
        if "entry" in result:
            print("  entry: " + (compact(result["entry"]) if not isinstance(result["entry"], str)
                                 else f'"{result["entry"]}"'))
        if result.get("error"):
            print("  error: " + result["error"])
        for w in result.get("companion_warnings", []):
            print("  • " + w)
        if result.get("homonym"):
            print("  homonym in: " + ", ".join(result["homonym"]))
        print("  next: " + "; ".join(result.get("next", [])))
    return result["exit_code"]

# ── Main ──────────────────────────────────────────────────────────────────────────────────────

def main():
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.normpath(os.path.join(here, ".."))
    default_lex = os.path.join(root, "crates", "logicaffeine_language", "assets", "lexicon.json")
    ap = argparse.ArgumentParser(description="Route a word to its lexicon section + companion edits.")
    ap.add_argument("--word")
    ap.add_argument("--lexicon", default=default_lex)
    ap.add_argument("--apply", action="store_true",
                    help="write the entry into lexicon.json automatically (safe data-adds only)")
    ap.add_argument("--no-apply", action="store_true", help="never offer to write; print only")
    ap.add_argument("--role", help="role slug for non-interactive mode (see --list-roles)")
    ap.add_argument("--spec", help="JSON object of fields for non-interactive mode")
    ap.add_argument("--json", action="store_true", help="machine-readable JSON output")
    ap.add_argument("--list-roles", action="store_true", help="print role slugs + sections and exit")
    ap.add_argument("--enrich", action="store_true",
                    help="surface WordNet/inflection evidence (plural, verb forms, candidate sorts, "
                         "hypernyms) — optional deps: inflect, lemminflect, nltk+wordnet")
    args = ap.parse_args()

    global ENUM
    ENUM, drift = build_enum_table(root)

    if args.list_roles:
        print(json.dumps({s: SLUG_SECTION[s] for s in ROLE_SLUGS}, indent=2))
        return 0

    try:
        lex = json.load(open(args.lexicon, encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        if args.role or args.spec:
            print(json.dumps({"status": "error", "error": f"cannot read lexicon: {e}",
                              "exit_code": EXIT_BAD_INPUT}))
            return EXIT_BAD_INPUT
        print(f"warning: could not read lexicon ({e}); conflict/homonym checks disabled.")
        lex = {}

    # Standalone evidence: `--word X --enrich [--json]` with no role -> just report evidence.
    if args.enrich and args.word and not (args.role or args.spec):
        if args.json:
            print(json.dumps({"word": args.word, "enrichment": lexical_evidence(args.word)},
                             indent=2, ensure_ascii=False))
            return 0
        print_evidence(args.word)
        if not sys.stdin.isatty():        # non-interactive caller: evidence only
            return 0

    if args.role or args.spec:
        args._drift = drift
        return run_noninteractive(args, lex, root)

    print("=" * 78)
    print(" place-word — where does this word belong, and what else needs wiring?")
    if drift:
        print(" ! enum drift / fallback: " + "; ".join(drift))
    print("=" * 78)
    word = args.word or ask_text("\nWhat is the word/expression")

    if args.enrich:
        print_evidence(word)

    occ = find_all_occurrences(lex, word)
    if occ:
        print(f"\n⚠ '{word}' ALREADY appears in: {', '.join(occ)}")
        print("  Adding another POS is a HOMONYM (changes parsing) — see the companion notes below.")

    role_label = choose("\nWhat role does it play IN THE SENTENCE?  (type ? if unsure)",
                        [r[0] for r in ROLES], help=HELP_ROLE)
    builder = next(r[2] for r in ROLES if r[0] == role_label)
    section, entry, note = globals()[builder](word, lex)
    if section == "__special__":
        print("This is wired in Rust (see above), not a lexicon.json word-add. Nothing to apply.")
        return
    if section is None:
        section = next(r[1] for r in ROLES if r[0] == role_label)

    top = section.split(".")[0]
    name = entry_name(entry, word)

    conflict = name in section_members(lex, section)
    homonym = bool([s for s in find_all_occurrences(lex, word) if s.split(".")[0] != top])

    print("\n" + "─" * 78)
    print(f"SECTION:  {section}")
    if conflict:
        print(f"CONFLICT: '{name}' already exists in `{section}` — check before adding "
              "(a duplicate/conflict is an escalation, not a silent overwrite).")

    print("\nADD THIS ENTRY:")
    if isinstance(entry, str):
        print(f'  "{entry}"   (append to the `{section}` array)')
    else:
        print("  " + compact(entry))
    if note:
        print(f"\nNOTE: {note}")

    advice = companion_edits(section, entry, word, lex, root)
    if advice:
        print("\nALSO WIRE (the second spots — a word is rarely one JSON line):")
        for a in advice:
            print("  • " + a)

    like = None
    if isinstance(entry, dict):
        if top == "nouns" and "sort" in entry:
            like = {"sort": entry["sort"]}
        elif top == "verbs" and "class" in entry:
            like = {"class": entry["class"]}
    sibs = siblings(lex, top, like=like) or siblings(lex, top)
    if sibs:
        print(f"\nMIRROR a sibling already in `{top}`:")
        for s in sibs:
            print("  " + compact(s))

    rel = os.path.relpath(args.lexicon, root)

    # ── Auto-apply: only when it's a genuinely safe pure data-add ──────────────────────────
    blockers = []
    if conflict:
        blockers.append("it already exists (conflict)")
    if homonym:
        blockers.append("it's a homonym — adding a POS changes parsing")
    if top == "keywords":
        blockers.append("keywords need token.rs + build.rs wiring")
    if not lex:
        blockers.append("lexicon could not be read")

    if blockers and not args.no_apply:
        print(f"\nNOT auto-applying ({'; '.join(blockers)}). Paste it yourself after handling that.")

    applied = False
    if entry is not None and not blockers and not args.no_apply:
        do = args.apply or ask_yn("\nApply this entry to lexicon.json now?", default=True)
        if do:
            ok, msg = apply_entry(args.lexicon, section, entry)
            print(f"\n{'✓ APPLIED — ' + msg if ok else '✗ could not apply: ' + msg}")
            applied = ok

    print("\nNEXT:")
    if applied:
        print(f"  1. ✓ written to {rel}")
        print("  2. cargo build   (regenerates the lexicon tables)")
    else:
        print(f"  1. Paste into the `{section}` section of {rel}")
        print("  2. cargo build   (regenerates the tables; `touch` the JSON if it looks stale)")
    print("  3. Re-trace / re-triage to confirm — no per-word test (see wikis/FIND_AND_FIX.md).")
    print("─" * 78)


if __name__ == "__main__":
    sys.exit(main() or 0)

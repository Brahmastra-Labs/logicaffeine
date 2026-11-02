//! The card's floating-point DSP block as the card measured it (`codegen_sva::fp_receipt`), held to the receipts ledger
//! `benchmarks/silicon/d5005/fp_hard_receipts.json`: every receipt reads again from the inputs kept beside it
//! (`fp_receipts/<build>-<measurement>/`), from a build that passed and the engine it compiled, and from the card's run
//! that configured its image. fp-oracle5's receipt says the card's add is the engine's own add, three registers deep,
//! which the engine's four-clock unit wraps with one register after it and nothing corrected; that its multiply is no
//! member of the family; and that its multiply-add rounds twice. The recorder refuses a receipt that any of its inputs
//! does not hold up, and a measurement it has recorded already.

use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::fp_hard::MeasureError;
use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
use logicaffeine_compile::codegen_sva::fp_oracle::{Oracle, OracleError};
use logicaffeine_compile::codegen_sva::fp_receipt::{
    engine_held, kept, kept_dir, receipt, receipt_of, record, CardRun, Inputs, Ledger, Near, Receipt, ReceiptError, Steps, Unit, KEPT, LEDGER_WHAT,
    NEAREST,
};
use logicaffeine_compile::crypto_accel::sha256;
use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};

fn d5005() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A receipt's kept inputs.
struct Kept {
    files: Vec<String>,
}

impl Kept {
    fn of(r: &Receipt) -> Kept {
        let dir = d5005().join("fp_receipts").join(kept_dir(r));
        Kept { files: KEPT.iter().map(|f| read(&dir.join(f))).collect() }
    }

    fn file(&self, name: &str) -> &str {
        &self.files[KEPT.iter().position(|f| *f == name).unwrap()]
    }

    fn card(&self) -> CardRun<'_> {
        CardRun {
            gbs_sha256: self.file("gbs.sha256"),
            pr: self.file("pr.log"),
            user_clock: self.file("userclk.log"),
            started: self.file("started"),
            finished: self.file("finished"),
            measurement: self.file("measurement.json"),
        }
    }
}

fn latency(unit: &Unit) -> usize {
    match unit {
        Unit::Hard { latency, .. } | Unit::Refused { latency, .. } => *latency,
    }
}

fn ledger() -> Ledger {
    serde_json::from_str(&read(&d5005().join("fp_hard_receipts.json"))).expect("the receipts ledger")
}

fn builds() -> String {
    read(&d5005().join("quartus_builds.json"))
}

fn oracle5() -> Receipt {
    ledger().receipts.into_iter().find(|r| r.build == "fp-oracle5").expect("fp-oracle5's receipt")
}

#[test]
fn every_receipt_reads_again_from_the_inputs_kept_beside_it() {
    let ledger = ledger();
    assert_eq!(ledger.what, LEDGER_WHAT);
    assert!(!ledger.receipts.is_empty());
    let builds = builds();
    for (i, r) in ledger.receipts.iter().enumerate() {
        let files = Kept::of(r);
        let dir = d5005().join("fp_receipts").join(kept_dir(r));
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        let mut want: Vec<String> = KEPT.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(names, want, "{}: the kept inputs and nothing else", r.build);
        for name in KEPT {
            assert_eq!(kept(name, files.file(name)), files.file(name), "{}: {name} kept as the recorder keeps it", r.build);
        }
        assert_eq!(hex(&sha256(files.file("measurement.json").as_bytes())), r.measurement_sha256, "{}", r.build);
        assert_eq!(hex(&sha256(files.file("discriminators.json").as_bytes())), r.discriminators_sha256, "{}", r.build);
        let inputs = Inputs {
            build: &r.build,
            builds: &builds,
            discriminators: files.file("discriminators.json"),
            image_sha256: &r.gbs_sha256,
            latencies: [latency(&r.multiply.unit), latency(&r.add.unit)],
            card: files.card(),
        };
        assert_eq!(receipt(&inputs, &r.recorded).as_ref(), Ok(r), "{}: the receipt its inputs make", r.build);
        assert_eq!(hex(&sha256(files.file("logos_engine.v").as_bytes())), r.engine_sha256, "{}: the engine kept is the one the build compiled", r.build);
        assert!(ledger.receipts[..i].iter().all(|o| o.measurement_sha256 != r.measurement_sha256), "{}: one receipt a measurement", r.build);
    }
}

#[test]
fn fp_oracle5_says_the_cards_add_is_the_engines_own_add_and_its_multiply_no_member_of_the_family() {
    let r = oracle5();
    assert_eq!(r.gbs_sha256, "984b31665a42f04d0588e3bae4ac3d00ec57b7a10604bb86e6bf961d8ab04657");
    assert_eq!(r.measurement_sha256, "19a6c0bbb318256fa27171b73f06694256821e4d37f69fb386f4ff52ea83e7f0");
    assert_eq!(r.user_clock_mhz, 100.0);
    assert_eq!(r.steps, Steps { vectors: 867, random: 1 << 24, near: 1 << 24 });
    assert_eq!((r.product_classes, r.sum_classes), (84, 36));

    let add = &r.add;
    assert_eq!((add.full, add.least, add.agreeing, add.unexplained), (3, 1, 1, 0));
    assert_eq!(add.class.first(), Some(&Semantics::ENGINE.name()), "the engine's own add: {:?}", add.class);
    assert_eq!(add.nearest.len(), NEAREST);
    assert_eq!(add.nearest[0], Near { class: Semantics::ENGINE.name(), differing: 0 });
    assert!(add.nearest[1].differing > 0, "the measurement tells the next class apart: {:?}", add.nearest);
    assert_eq!(
        add.unit,
        Unit::Hard { latency: FADD_LATENCY, registering: "full".into(), pads: 1, flush_operands: false, flush_results: false, canonical_nan: false },
        "registered fully, one register after it, nothing corrected"
    );

    let mul = &r.multiply;
    assert_eq!((mul.full, mul.least, mul.agreeing, mul.unexplained), (4, 1, 0, 0), "every step explained by some class, no class by every step");
    assert!(mul.class.is_empty());
    assert!(mul.nearest[0].differing > 0, "{:?}", mul.nearest);
    match &mul.unit {
        Unit::Refused { latency, why } => {
            assert_eq!(*latency, FMUL_LATENCY);
            assert!(why.contains("no member of the family"), "{why}");
        }
        u => panic!("a multiply no class names has no unit: {u:?}"),
    }
    assert_eq!((r.multiply_add_latency, r.multiply_add_rounding.as_str()), (5, "twice"));
}

#[test]
fn the_receipt_names_the_build_that_compiled_the_engine_the_card_ran() {
    let r = oracle5();
    let builds: serde_json::Value = serde_json::from_str(&builds()).unwrap();
    let row = builds["rows"].as_array().unwrap().iter().find(|b| b["name"] == r.build.as_str()).expect("the build's row");
    assert_eq!(row["verdict"]["passed"], true);
    let engine: Vec<&str> = row["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["path"].as_str().unwrap().ends_with("/hw/rtl/logos_engine.v"))
        .map(|i| i["sha256"].as_str().unwrap())
        .collect();
    assert_eq!(engine, vec![r.engine_sha256.as_str()]);
    assert!(Kept::of(&r).file("gbs.sha256").starts_with(&format!("{}  ", r.gbs_sha256)), "the image the run configured");
    assert!(r.started <= r.finished && r.finished <= r.recorded, "{} {} {}", r.started, r.finished, r.recorded);
}

/// fp-oracle5's inputs with `change` made to them, read.
fn changed(change: impl FnOnce(&mut Vec<String>, &mut String, &mut String, &mut String)) -> Result<Receipt, ReceiptError> {
    let r = oracle5();
    let mut kept = Kept::of(&r);
    let (mut builds, mut build, mut image) = (builds(), r.build.clone(), r.gbs_sha256.clone());
    change(&mut kept.files, &mut builds, &mut build, &mut image);
    let inputs = Inputs {
        build: &build,
        builds: &builds,
        discriminators: kept.file("discriminators.json"),
        image_sha256: &image,
        latencies: [FMUL_LATENCY, FADD_LATENCY],
        card: kept.card(),
    };
    receipt(&inputs, &r.recorded)
}

fn at(name: &str) -> usize {
    KEPT.iter().position(|f| *f == name).unwrap()
}

/// The build ledger with fp-oracle5's row changed by `change`.
fn row_changed(builds: &str, change: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut v: serde_json::Value = serde_json::from_str(builds).unwrap();
    let row = v["rows"].as_array_mut().unwrap().iter_mut().find(|b| b["name"] == "fp-oracle5").unwrap();
    change(row);
    serde_json::to_string(&v).unwrap()
}

#[test]
fn a_receipt_is_refused_when_any_of_its_inputs_does_not_hold_up() {
    assert!(changed(|_, _, _, _| {}).is_ok(), "the inputs as kept");
    let e = changed(|_, _, _, image| image.replace_range(..1, "0")).unwrap_err();
    assert!(matches!(e, ReceiptError::Image { .. }), "another image than the run configured: {e}");
    let e = changed(|k, _, _, _| k[at("pr.log")] = k[at("pr.log")].replace("CRC 0", "CRC 1")).unwrap_err();
    assert!(matches!(&e, ReceiptError::Reconfiguration(w) if w.contains("CRC 1")), "{e}");
    let e = changed(|k, _, _, _| k[at("userclk.log")].clear()).unwrap_err();
    assert_eq!(e, ReceiptError::UserClock);
    let e = changed(|k, _, _, _| k.swap(at("started"), at("finished"))).unwrap_err();
    assert!(matches!(e, ReceiptError::Times { .. }), "finished before it started: {e}");
    let e = changed(|k, _, _, _| k[at("started")] = k[at("started")].replace('T', " ")).unwrap_err();
    assert!(matches!(e, ReceiptError::Times { .. }), "a time in another shape: {e}");
    let e = changed(|k, _, _, _| k[at("measurement.json")] = k[at("measurement.json")].replace("\"complete\": true", "\"complete\": false")).unwrap_err();
    assert_eq!(e, ReceiptError::Measure(MeasureError::Incomplete("finished run")));
    let e = changed(|_, _, build, _| *build = "fp-oracle-none".into()).unwrap_err();
    assert_eq!(e, ReceiptError::NoBuild("fp-oracle-none".into()));
    let e = changed(|_, b, _, _| *b = row_changed(b, |row| row["verdict"]["passed"] = false.into())).unwrap_err();
    assert_eq!(e, ReceiptError::BuildFailed("fp-oracle5".into()));
    let e = changed(|_, b, _, _| {
        *b = row_changed(b, |row| row["inputs"].as_array_mut().unwrap().retain(|i| !i["path"].as_str().unwrap().ends_with("logos_engine.v")))
    })
    .unwrap_err();
    assert_eq!(e, ReceiptError::NoEngine("fp-oracle5".into()));
    let e = changed(|_, b, _, _| {
        *b = row_changed(b, |row| {
            let inputs = row["inputs"].as_array_mut().unwrap();
            let mut second = inputs.iter().find(|i| i["path"].as_str().unwrap().ends_with("logos_engine.v")).unwrap().clone();
            second["path"] = "/mnt/build/d5005-afu/other/afu/hw/rtl/logos_engine.v".into();
            inputs.push(second);
        })
    })
    .unwrap_err();
    assert_eq!(e, ReceiptError::NoEngine("fp-oracle5".into()), "two engines name no one engine the build compiled");
    let e = changed(|_, b, _, _| *b = "{".into()).unwrap_err();
    assert!(matches!(e, ReceiptError::Ledger(_)), "{e}");
    let e = changed(|k, _, _, _| k[at("discriminators.json")] = "[]".into()).unwrap_err();
    assert_eq!(e, ReceiptError::Oracle(OracleError::Missing("fmul")));
}

#[test]
fn the_measurement_kept_holds_the_card_to_the_class_it_named() {
    let r = oracle5();
    let kept = Kept::of(&r);
    let mut v: serde_json::Value = serde_json::from_str(kept.file("measurement.json")).unwrap();
    let engine = r.add.nearest[0].class.clone();
    let oracle = Oracle::from_ledger(kept.file("discriminators.json")).unwrap();
    let k = oracle.sums.iter().position(|c| c.members[0] == engine).unwrap();
    v["runs"]["near"]["sums"][k] = 1.into();
    let text = serde_json::to_string(&v).unwrap();
    let e = changed(|f, _, _, _| f[at("measurement.json")] = text).unwrap();
    assert_eq!((e.add.agreeing, e.add.class.len()), (0, 0), "one step against it and no class is the add");
    assert!(matches!(&e.add.unit, Unit::Refused { why, .. } if why.contains("no member of the family")), "{:?}", e.add.unit);
    assert_eq!(e.add.nearest[0], Near { class: engine, differing: 1 });
}

/// fp-oracle5's inputs, recorded into `ledger` against the build ledger whose fp-oracle5 row names the engine `engine`,
/// with the engine the kept discriminators make today standing in for a build compiled today.
fn recorded(ledger: Option<&str>, engine: &str) -> Result<(Receipt, String), ReceiptError> {
    let r = oracle5();
    let kept = Kept::of(&r);
    let builds = row_changed(&builds(), |row| {
        for i in row["inputs"].as_array_mut().unwrap() {
            if i["path"].as_str().unwrap().ends_with("/hw/rtl/logos_engine.v") {
                i["sha256"] = engine.into();
            }
        }
    });
    let inputs = Inputs {
        build: &r.build,
        builds: &builds,
        discriminators: kept.file("discriminators.json"),
        image_sha256: &r.gbs_sha256,
        latencies: [FMUL_LATENCY, FADD_LATENCY],
        card: kept.card(),
    };
    let today = Oracle::from_ledger(kept.file("discriminators.json")).unwrap().engine_verilog();
    record(ledger, &inputs, &today, &r.recorded)
}

#[test]
fn recording_appends_a_receipt_whose_discriminators_regenerate_the_engine_and_never_one_recorded_already() {
    let kept = Kept::of(&oracle5());
    let regenerated = hex(&sha256(Oracle::from_ledger(kept.file("discriminators.json")).unwrap().engine_verilog().as_bytes()));
    let (r, text) = recorded(None, &regenerated).expect("recorded into a new ledger");
    assert_eq!(r.engine_sha256, regenerated);
    let fresh: Ledger = serde_json::from_str(&text).unwrap();
    assert_eq!(fresh, Ledger { what: LEDGER_WHAT.into(), receipts: vec![r.clone()] });
    assert!(text.ends_with("}\n") && text.contains("\n \"receipts\": ["), "written as the other ledgers are, one space a level");

    let wrong = "0".repeat(64);
    let e = recorded(None, &wrong).unwrap_err();
    assert_eq!(e, ReceiptError::Engine { build: "fp-oracle5".into(), compiled: wrong, kept: regenerated.clone() });

    let e = recorded(Some(&text), &regenerated).unwrap_err();
    assert_eq!(e, ReceiptError::Duplicate(r.measurement_sha256.clone()));

    let mut other = r.clone();
    other.measurement_sha256 = "1".repeat(64);
    other.build = "fp-oracle4".into();
    let before = serde_json::to_string(&Ledger { what: LEDGER_WHAT.into(), receipts: vec![other.clone()] }).unwrap();
    let (_, after) = recorded(Some(&before), &regenerated).expect("appended");
    let after: Ledger = serde_json::from_str(&after).unwrap();
    assert_eq!(after.receipts, vec![other, r], "appended after what the ledger held");

    let e = recorded(Some("{"), &regenerated).unwrap_err();
    assert!(matches!(e, ReceiptError::Ledger(_)), "{e}");
}

/// A hard-float build takes its blocks from a recorded card run: the receipt that records a measurement, found by the
/// measurement's SHA-256, whose kept discriminator ledger is the oracle it was measured against; a measurement no
/// receipt records has none.
#[test]
fn a_measurement_is_read_against_the_discriminators_its_receipt_kept() {
    let ledger = ledger();
    let r = oracle5();
    let kept = Kept::of(&r);
    let found = receipt_of(&ledger, kept.file("measurement.json")).expect("fp-oracle5's measurement is recorded");
    assert_eq!(found, &r);
    let oracle = Oracle::from_ledger(kept.file("discriminators.json")).unwrap();
    assert_eq!((oracle.products.len(), oracle.sums.len()), (r.product_classes, r.sum_classes), "the oracle it was measured with");
    let sum = logicaffeine_compile::codegen_sva::fp_hard::Measurement::read_sum(&oracle, kept.file("measurement.json")).unwrap();
    assert_eq!(sum.semantics, Semantics::ENGINE);
    let other = kept.file("measurement.json").replace("\"seed\": ", "\"seed\":  ");
    assert_eq!(receipt_of(&ledger, &other), None, "a measurement no receipt records");
}

/// A receipt is recorded only for the engine the build compiled, by its SHA-256, of which every module the oracle writes
/// itself (`Oracle::own_verilog`: its units, vector ROM, delay line and core) the discriminators regenerate byte for byte
/// as the code writes them when it is recorded. The library it embeds (the engine's crossing and top, the store's memory)
/// is held by the build's SHA-256 alone, so the library may move on between a build and its receipt; an own module that
/// does not regenerate, or is missing, is refused by name. The engine fp-oracle5's discriminators make today stands in
/// for a build compiled today.
#[test]
fn a_receipt_holds_the_oracles_own_modules_and_lets_the_library_it_embeds_move_on() {
    let r = oracle5();
    let kept = Kept::of(&r);
    let discriminators = kept.file("discriminators.json");
    let today = Oracle::from_ledger(discriminators).unwrap().engine_verilog();
    let engine = today.as_str();
    // The engine with `to` written after the name on module `name`'s line, that name alone and not one it begins.
    let edit = |name: &str, to: &str| -> String {
        let line = format!("module {name}");
        let (at, _) = engine
            .match_indices(&line)
            .find(|(i, _)| !engine[i + line.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or_else(|| panic!("{name} is in the engine"));
        let end = at + line.len();
        format!("{}{to}{}", &engine[..end], &engine[end..])
    };
    let compiled_as = |text: &str| {
        let sha = hex(&sha256(text.as_bytes()));
        row_changed(&builds(), |row| {
            for i in row["inputs"].as_array_mut().unwrap() {
                if i["path"].as_str().unwrap().ends_with("/hw/rtl/logos_engine.v") {
                    i["sha256"] = sha.clone().into();
                }
            }
        })
    };
    assert_eq!(engine_held(&r.build, &compiled_as(engine), discriminators, engine), Ok(()), "a build of today's engine");
    for library in ["logos_ram_ce_w512", "logos_engine_bridge", "logos_engine"] {
        let moved = edit(library, " /* as another library wrote it */");
        assert_eq!(engine_held(&r.build, &compiled_as(&moved), discriminators, &moved), Ok(()), "{library}: the library's, held by the build's SHA alone");
        assert_eq!(
            engine_held(&r.build, &compiled_as(engine), discriminators, &moved),
            Err(ReceiptError::Engine { build: "fp-oracle5".into(), compiled: hex(&sha256(engine.as_bytes())), kept: hex(&sha256(moved.as_bytes())) }),
            "{library}: an engine the build did not compile"
        );
    }
    for own in ["logos_fpo_mul0", "logos_fpo_add35", "logos_fpo_vectors", "logos_fpo_late", "logos_fp_oracle"] {
        let strayed = edit(own, " /* strayed */");
        assert_eq!(
            engine_held(&r.build, &compiled_as(&strayed), discriminators, &strayed),
            Err(ReceiptError::Regenerated { build: "fp-oracle5".into(), module: own.into() }),
            "{own}: the oracle's own, which the discriminators must regenerate"
        );
    }
    let missing = edit("logos_fpo_late", "r");
    assert_eq!(
        engine_held(&r.build, &compiled_as(&missing), discriminators, &missing),
        Err(ReceiptError::Regenerated { build: "fp-oracle5".into(), module: "logos_fpo_late".into() }),
        "an own module the engine lacks"
    );
    let own = Oracle::from_ledger(discriminators).unwrap().own_verilog();
    let modules = own.lines().filter(|l| l.starts_with("module ")).count();
    assert_eq!(modules, r.product_classes + r.sum_classes + 3, "a unit a class, the vectors, the delay line and the core");
}

//! Record what the card measured of its floating-point DSP block (`codegen_sva::fp_receipt`): `<card run>` is the
//! directory the oracle persona's run on the card wrote (`gbs.sha256`, `pr.log`, `userclk.log`, `started`, `finished`,
//! `measurement.json`), `<image>` the image the build compiled, `<engine>` its compiled `logos_engine.v`, `<build>` its
//! row in `<build ledger>`, `<discriminators>` the discriminator ledger its oracle was made from. The receipt is appended to `<receipts ledger>` at the units'
//! latencies the engine has now, and its inputs are kept beside it in `fp_receipts/<build>-<measurement>/` (the image's
//! `sha256sum` line naming it by its file name alone). It exits 1 naming why when the receipt is refused, and refuses a
//! kept-inputs directory that is already there.
//!
//!     d5005_fp_receipt <card run> <image> <engine> <build> <discriminators> <build ledger> <receipts ledger>

use std::path::Path;
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use logicaffeine_base::temporal::civil_from_days;
use logicaffeine_compile::codegen_sva::fp_receipt::{kept, kept_dir, record, CardRun, Inputs, KEPT};
use logicaffeine_compile::crypto_accel::sha256;
use logicaffeine_compile::spatial::decode_engine::rtl::fp32::{FADD_LATENCY, FMUL_LATENCY};

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_fp_receipt: refused: {why}");
    exit(1)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())))
}

/// Now, `YYYY-MM-DDTHH:MM:SSZ`.
fn now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_secs() as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [run, image, engine, build, discriminators, builds, ledger] = &args[..] else {
        fail("usage: d5005_fp_receipt <card run> <image> <engine> <build> <discriminators> <build ledger> <receipts ledger>")
    };
    let engine = read(Path::new(engine));
    let run = Path::new(run);
    let card_file = |name: &str| read(&run.join(name));
    let (gbs, pr, clock, started, finished, measurement) =
        (card_file("gbs.sha256"), card_file("pr.log"), card_file("userclk.log"), card_file("started"), card_file("finished"), card_file("measurement.json"));
    let image_sha256: String =
        sha256(&std::fs::read(image).unwrap_or_else(|e| fail(format!("{image}: {e}")))).iter().map(|b| format!("{b:02x}")).collect();
    let (discriminators, builds) = (read(Path::new(discriminators)), read(Path::new(builds)));
    let ledger = Path::new(ledger);
    let held = ledger.exists().then(|| read(ledger));
    let inputs = Inputs {
        build,
        builds: &builds,
        discriminators: &discriminators,
        image_sha256: &image_sha256,
        latencies: [FMUL_LATENCY, FADD_LATENCY],
        card: CardRun { gbs_sha256: &gbs, pr: &pr, user_clock: &clock, started: &started, finished: &finished, measurement: &measurement },
    };
    let (receipt, text) = record(held.as_deref(), &inputs, &engine, &now()).unwrap_or_else(|e| fail(e));
    let dir = ledger.parent().unwrap_or(Path::new(".")).join("fp_receipts").join(kept_dir(&receipt));
    if dir.exists() {
        fail(format!("{} is already there", dir.display()));
    }
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(format!("{}: {e}", dir.display())));
    for (name, text) in KEPT.iter().zip([&measurement, &discriminators, &gbs, &pr, &clock, &started, &finished, &engine]) {
        std::fs::write(dir.join(name), kept(name, text)).unwrap_or_else(|e| fail(format!("{name}: {e}")));
    }
    std::fs::write(ledger, text).unwrap_or_else(|e| fail(format!("{}: {e}", ledger.display())));
    println!(
        "d5005_fp_receipt: recorded {} on {}: add {}, multiply {}, multiply-add {} deep rounding {}",
        receipt.build,
        &receipt.gbs_sha256[..12],
        receipt.add.class.first().map_or("no class", String::as_str),
        receipt.multiply.class.first().map_or("no class", String::as_str),
        receipt.multiply_add_latency,
        receipt.multiply_add_rounding
    );
}

use super::super::ddr2_jedec::{Ddr2Grade, Ddr2Timing};
use super::super::ddr2_phy::{ddr2_pins, Ddr2Cmd};
use super::*;
use std::collections::BTreeSet;

fn ddr400(bl: u64) -> Ddr2Timing {
    Ddr2Grade::MT47H64M16_25.at(K7_TCK_PS, bl).unwrap()
}

fn seq(words: u32) -> Traffic {
    Traffic { pattern: Pattern::Sequential, words }
}

fn scattered(words: u32) -> Traffic {
    Traffic { pattern: Pattern::Scattered, words }
}

#[test]
fn the_operating_point_is_the_boards_part_at_ddr2_400() {
    assert_eq!(K7_TCK_PS, 5000);
    assert_eq!(K7_GRADE, Ddr2Grade::MT47H64M16_25);
    let t = K7_GRADE.at(K7_TCK_PS, 8).unwrap();
    assert_eq!((t.cl, t.wl), (3, 2), "CL 3 at 400 MT/s");
}

/// LiteDRAM's ROW_BANK_COL on the native port's address (column bits above the burst, then bank, then row): a word
/// index IS a native address, and its four x16 columns start at 4 × its column word.
#[test]
fn a_word_lives_where_litedram_puts_it() {
    assert_eq!(place(0), Place { bank: 0, row: 0, col: 0 });
    assert_eq!(place(1), Place { bank: 0, row: 0, col: 4 });
    assert_eq!(place(255), Place { bank: 0, row: 0, col: 1020 });
    assert_eq!(place(256), Place { bank: 1, row: 0, col: 0 });
    assert_eq!(place(2047), Place { bank: 7, row: 0, col: 1020 });
    assert_eq!(place(2048), Place { bank: 0, row: 1, col: 0 });
    assert_eq!(place((1 << 24) - 1), Place { bank: 7, row: 8191, col: 1020 });
    for w in [0u32, 7, 300, 4095, 77_777, 1 << 20] {
        let p = place(w);
        assert_eq!((u32::from(p.row) << 11) | (u32::from(p.bank) << 8) | u32::from(p.col / 4), w, "the bit fields recompose word {w}");
        assert_eq!(p.col % 4, 0);
    }
}

#[test]
fn a_word_past_the_part_is_refused() {
    assert!(std::panic::catch_unwind(|| place(1 << 24)).is_err(), "128 MiB is 2^24 words");
}

#[test]
fn the_sequential_stream_is_every_word_in_order() {
    assert_eq!(seq(5).order(), vec![0, 1, 2, 3, 4]);
    assert_eq!(seq(3000).order().len(), 3000);
    assert_eq!(seq(3000).rows(), BTreeSet::from([(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0), (0, 1), (1, 1), (2, 1), (3, 1)]));
}

/// The scattered stream: distinct words drawn from a fixed region of rows and banks, in an order that changes row
/// and bank nearly every request — the traffic a row-hit scheduler cannot help.
#[test]
fn the_scattered_stream_is_distinct_words_across_rows_and_banks() {
    let t = scattered(1024);
    let order = t.order();
    assert_eq!(order, scattered(1024).order(), "deterministic");
    assert_eq!(order.len(), 1024);
    assert_eq!(order.iter().collect::<BTreeSet<_>>().len(), 1024, "no word twice");
    for &w in &order {
        assert!(place(w).row < SCATTER_ROWS as u16, "word {w} inside the region");
    }
    let rows = t.rows();
    assert!(rows.len() > 300, "{} (bank, row) pairs", rows.len());
    let changes = order.windows(2).filter(|p| (place(p[0]).bank, place(p[0]).row) != (place(p[1]).bank, place(p[1]).row)).count();
    assert!(changes > 1000, "{changes} of 1023 consecutive pairs change row or bank");
    assert_ne!(order, { let mut s = order.clone(); s.sort(); s }, "not in address order");
    assert_eq!(scattered(16).order(), scattered(1024).order()[..16], "a shorter stream is a prefix");
    assert_eq!(t.row_bits(), 6, "64 rows a bank");
    assert_eq!(seq(2).row_bits(), 1, "row 0 still needs a bit");
    assert_eq!(seq(4096).row_bits(), 1, "rows 0 and 1");
    assert_eq!(seq(4097).row_bits(), 2, "row 2");
}

#[test]
fn every_word_carries_its_own_value() {
    let values: BTreeSet<u64> = seq(4096).order().iter().map(|&w| word_value(w)).collect();
    assert_eq!(values.len(), 4096);
    assert_ne!(word_value(0), 0);
    assert_eq!(word_value(1), word_value(1));
}

#[test]
fn the_rom_is_one_word_and_value_per_line() {
    let t = scattered(8);
    let rom = t.rom();
    let lines: Vec<&str> = rom.lines().collect();
    assert_eq!(lines.len(), 8);
    for (line, w) in lines.iter().zip(t.order()) {
        assert_eq!(*line, format!("{w:06x}{:016x}", word_value(w)));
    }
}

#[test]
fn patterns_have_names() {
    for p in [Pattern::Sequential, Pattern::Scattered, Pattern::Probe] {
        assert_eq!(Pattern::parse(p.label()), Some(p));
    }
    assert_eq!(Pattern::parse("random"), None);
}

/// The probe reads the scattered words, one every PROBE_EVERY clocks; the streams present each read as soon as the
/// one before is taken.
#[test]
fn the_probe_spaces_the_scattered_words() {
    let probe = Traffic { pattern: Pattern::Probe, words: 64 };
    assert_eq!(probe.order(), scattered(64).order());
    assert_eq!(probe.every(), PROBE_EVERY);
    assert_eq!(seq(64).every(), 0);
    assert_eq!(scattered(64).every(), 0);
    assert!(PROBE_EVERY >= 2 * 64, "room for a read and a refresh in its way");
}

// ── the log ──────────────────────────────────────────────────────────────────────────────

fn c(cycle: u64, cmd: Ddr2Cmd, ba: u8, a: u16) -> String {
    let (cs, ras, cas, we) = ddr2_pins(cmd);
    format!("C {cycle} {cs} {ras} {cas} {we} {ba} {a}\n")
}

#[test]
fn the_log_reads_back() {
    let text = format!(
        "{}P 0 W 5 2\nQ 1 W 5 2\n\nD 9 5 {:016x}\n{}END 12 0\n",
        c(3, Ddr2Cmd::Active, 2, 17),
        0xdead_beef_u64,
        c(6, Ddr2Cmd::Write, 2, 4)
    );
    let log = read_log(&text).unwrap();
    assert_eq!(log.bus.len(), 2);
    assert_eq!((log.bus[0].cycle, log.bus[0].cmd, log.bus[0].ba, log.bus[0].a), (3, Ddr2Cmd::Active, 2, 17));
    assert_eq!(log.bus[1].cmd, Ddr2Cmd::Write);
    assert_eq!(log.presented, vec![Request { cycle: 0, kind: Kind::Write, word: 5, part: 2 }]);
    assert_eq!(log.requests, vec![Request { cycle: 1, kind: Kind::Write, word: 5, part: 2 }]);
    assert_eq!(log.deliveries, vec![Delivery { cycle: 9, word: 5, value: 0xdead_beef }]);
    assert_eq!(log.end, Some((12, 0)));
}

#[test]
fn a_malformed_log_line_is_named() {
    for (bad, why) in [
        ("X 1 2\n", "an unknown tag"),
        ("C 1 0 0 1\n", "too few pin fields"),
        ("C 1 0 1 1 0 0 0\n", "the reserved pin pattern"),
        ("Q 1 V 0 0\n", "a request neither W nor R"),
        ("P 1 W 0\n", "a presentation without its part"),
        ("Q x W 0 0\n", "a cycle that is not a number"),
        ("D 1 2 zz\n", "a value that is not hex"),
        ("END 1\n", "an END without its error count"),
        ("END 1 0\nEND 2 0\n", "two ENDs"),
    ] {
        let e = read_log(bad).unwrap_err();
        assert!(e.contains("line "), "{why}: {e}");
    }
}

// ── the judgement ────────────────────────────────────────────────────────────────────────

/// A two-word stream through a BL4 controller at its minimums: both words written into one open row, read back,
/// the row closed.
fn tiny_log() -> String {
    let (v0, v1) = (word_value(0), word_value(1));
    let mut s = String::new();
    s += "P 0 W 0 0\nQ 0 W 0 0\nP 1 W 1 0\nQ 1 W 1 0\n";
    s += &c(2, Ddr2Cmd::Active, 0, 0);
    s += &c(5, Ddr2Cmd::Write, 0, 0);
    s += &c(7, Ddr2Cmd::Write, 0, 4);
    s += "P 8 R 0 0\nQ 8 R 0 0\nP 9 R 1 0\nQ 9 R 1 0\n";
    s += &c(13, Ddr2Cmd::Read, 0, 0);
    s += &c(15, Ddr2Cmd::Read, 0, 4);
    s += &format!("D 16 0 {v0:016x}\n");
    s += &c(17, Ddr2Cmd::Precharge, 0, 0);
    s += &format!("D 18 1 {v1:016x}\n");
    s += "END 20 0\n";
    s
}

const WORDWISE: Interface = Interface { parts: 1, cols_per_part: 4 };

#[test]
fn a_clean_run_is_measured() {
    let j = judge(&ddr400(4), &seq(2), WORDWISE, &read_log(&tiny_log()).unwrap()).unwrap();
    assert_eq!(j.words, 2);
    assert_eq!(j.requests, 4);
    assert_eq!((j.bus.activates, j.bus.writes, j.bus.reads, j.bus.precharges, j.bus.refreshes), (1, 2, 2, 1, 0));
    assert_eq!(j.write_latency, Stats { n: 2, min: 5, max: 6, p50: 5, p99: 6, mean: 5.5 });
    assert_eq!(j.read_latency, Stats { n: 2, min: 5, max: 6, p50: 5, p99: 6, mean: 5.5 });
    assert_eq!(j.read_service, Stats { n: 2, min: 8, max: 9, p50: 8, p99: 9, mean: 8.5 }, "presented at 8 and 9, delivered at 16 and 18");
    assert_eq!(j.read_bus, Stats { n: 2, min: 10, max: 11, p50: 10, p99: 11, mean: 10.5 }, "the READs at 13 and 15, CL 3, BL4: last beats end at 18 and 20");
    assert_eq!(j.stream_cycles, 18, "first request at 0, last word delivered at 18");
    assert_eq!(j.payload_cycles, 8, "two words each way, two x16 DDR clocks a word");
    assert_eq!(j.bus.data_cycles, 8);
    assert_eq!(j.bus_span, 18, "the first ACT at 2 to the last beat of the last read at 19");
    assert_eq!(j.row_hits, 3, "four column commands, one ACT");
}

fn refused(log: &str) -> String {
    match read_log(log) {
        Err(e) => e,
        Ok(l) => judge(&ddr400(4), &seq(2), WORDWISE, &l).map(|_| ()).unwrap_err(),
    }
}

#[test]
fn a_run_that_breaks_a_bus_rule_is_refused() {
    let log = tiny_log().replace(&c(13, Ddr2Cmd::Read, 0, 0), &c(12, Ddr2Cmd::Read, 0, 0));
    let e = refused(&log);
    assert!(e.contains("WRITE to READ"), "{e}");
}

#[test]
fn a_wrong_value_is_refused() {
    let log = tiny_log().replace(&format!("D 18 1 {:016x}", word_value(1)), &format!("D 18 1 {:016x}", word_value(1) ^ 4));
    let e = refused(&log);
    assert!(e.contains("word 1") && e.contains("read back"), "{e}");
}

#[test]
fn a_missing_or_doubled_delivery_is_refused() {
    let log = tiny_log().replace(&format!("D 18 1 {:016x}\n", word_value(1)), "");
    assert!(refused(&log).contains("delivered"), "{}", refused(&log));
    let log = tiny_log().replace(&format!("D 18 1 {:016x}\n", word_value(1)), &format!("D 18 0 {:016x}\n", word_value(0)));
    assert!(refused(&log).contains("word 0"), "{}", refused(&log));
}

#[test]
fn a_request_that_never_reached_the_bus_is_refused() {
    let log = tiny_log().replace(&c(15, Ddr2Cmd::Read, 0, 4), &c(15, Ddr2Cmd::Read, 0, 8));
    let e = refused(&log);
    assert!(e.contains("word 1") && e.contains("never reached the bus"), "{e}");
}

#[test]
fn a_column_command_before_its_request_does_not_serve_it() {
    let log = tiny_log().replace("Q 9 R 1 0\n", "").replace("Q 8 R 0 0\nP 9 R 1 0\n", "Q 8 R 0 0\nP 9 R 1 0\nQ 16 R 1 0\n");
    let e = refused(&log);
    assert!(e.contains("word 1") && e.contains("never reached the bus"), "a READ at 15 cannot serve a request accepted at 16: {e}");
}

#[test]
fn requests_out_of_stream_order_are_refused() {
    let log = tiny_log().replace("Q 0 W 0 0\nP 1 W 1 0\nQ 1 W 1 0\n", "Q 0 W 1 0\nP 1 W 1 0\nQ 1 W 0 0\n");
    let e = refused(&log);
    assert!(e.contains("request 0 accepted"), "{e}");
    let log = tiny_log().replace("Q 9 R 1 0\n", "");
    assert!(refused(&log).contains("3 requests accepted"), "{}", refused(&log));
    let log = tiny_log().replace("P 9 R 1 0\n", "");
    assert!(refused(&log).contains("3 requests presented"), "{}", refused(&log));
}

#[test]
fn a_request_accepted_before_it_was_presented_is_refused() {
    let log = tiny_log().replace("P 9 R 1 0\n", "P 10 R 1 0\n");
    let e = refused(&log);
    assert!(e.contains("request 3") && e.contains("before it was presented"), "{e}");
}

#[test]
fn a_word_delivered_before_its_read_was_presented_is_refused() {
    let log = tiny_log().replace(&format!("D 18 1 {:016x}", word_value(1)), &format!("D 8 1 {:016x}", word_value(1)));
    let e = refused(&log);
    assert!(e.contains("word 1 delivered at cycle 8") && e.contains("presented at 9"), "{e}");
}

#[test]
fn an_activate_outside_the_stream_is_refused() {
    let log = tiny_log().replace("END 20 0\n", &format!("{}END 30 0\n", c(24, Ddr2Cmd::Active, 3, 40)));
    let e = refused(&log);
    assert!(e.contains("bank 3 row 40"), "{e}");
}

#[test]
fn an_unfinished_or_failing_bench_is_refused() {
    assert!(refused(&tiny_log().replace("END 20 0\n", "")).contains("did not finish"));
    assert!(refused(&tiny_log().replace("END 20 0\n", "END 20 3\n")).contains("3 errors"));
}

#[test]
fn a_controller_that_splits_a_word_is_matched_part_by_part() {
    // A 16-bit request port: each word is four requests, part p at column 4 × (column word) + p, BL8.
    let t = ddr400(8);
    let iface = Interface { parts: 4, cols_per_part: 1 };
    let v = word_value(0);
    let mut s = String::new();
    let mut cyc = 0u64;
    for (kind, cmd) in [("W", Ddr2Cmd::Write), ("R", Ddr2Cmd::Read)] {
        for p in 0..4u16 {
            s += &format!("P {cyc} {kind} 0 {p}\nQ {cyc} {kind} 0 {p}\n");
            s += &c(cyc + 1, Ddr2Cmd::Active, 0, 0);
            s += &c(cyc + 1 + t.trcd, cmd, 0, p);
            let pre = cyc + 1 + (t.trcd + t.write_to_precharge()).max(t.tras);
            s += &c(pre, Ddr2Cmd::Precharge, 0, 0);
            cyc = pre + t.trp;
        }
    }
    s += &format!("D {cyc} 0 {v:016x}\nEND {} 0\n", cyc + 1);
    let j = judge(&t, &seq(1), iface, &read_log(&s).unwrap()).unwrap();
    assert_eq!(j.requests, 8);
    assert_eq!(j.write_latency.min, 1 + t.trcd);
    assert_eq!(j.row_hits, 0, "closed page: every column command its own ACT");
}

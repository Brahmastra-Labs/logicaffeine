use super::*;

/// The helper emits exactly `slice` `(* keep *)` copies of the select bit, each in a `{G{...}}`
/// replication of `G = width / slice` bits — the structural fan-out bound. Checked at two
/// (width, slice) points: (64, 8) → G = 8, and (32, 4) → G = 8.
#[test]
fn bounded_fanout_select_emits_slice_keep_copies_each_gating_a_group() {
    for &(width, slice) in &[(64usize, 8usize), (32usize, 4usize)] {
        let group = width / slice;
        let (decls, pp) = bounded_fanout_select("msel", "", "sel[0]", "data", width, slice);

        // Exactly `slice` `(* keep *)` copies, one per line, named `mselK`, driven by `sel[0]`.
        let copies = decls
            .lines()
            .filter(|l| l.contains("(* keep *) wire msel") && l.contains("= sel[0];"))
            .count();
        assert_eq!(
            copies, slice,
            "must emit EXACTLY {slice} `(* keep *)` select copies, found {copies}"
        );
        for s in 0..slice {
            assert!(
                decls.contains(&format!("(* keep *) wire msel{s} = sel[0];")),
                "copy {s} must be a `(* keep *)` wire driven by the select bit"
            );
        }
        // No extra copy beyond the slice.
        assert!(
            !decls.contains(&format!("wire msel{slice} =")),
            "no copy `msel{slice}` beyond the {slice}-wide slice"
        );

        // Each copy drives a `{G{mselK}}` replication of EXACTLY `G = width / slice` bits.
        for s in 0..slice {
            assert!(
                pp.contains(&format!("{{{group}{{msel{s}}}}}")),
                "select copy {s} must drive EXACTLY {group} mux bits via `{{{group}{{msel{s}}}}}`"
            );
        }
        // The un-sliced FULL-width replication (the high-fan-out antenna net) must be ABSENT.
        assert!(
            !pp.contains(&format!("{{{width}{{msel0}}}}")),
            "no copy may drive the FULL {width}-bit datapath"
        );
        // The concatenation is high→low: group 0 (the LSB group, `data[G-1:0]`) is the LAST
        // term. Exactly one term ends at the LSB (`:0]`), and it is the last-emitted one — so
        // the groups tile the data wire contiguously with group 0 landing at the least bit.
        assert_eq!(
            pp.matches(":0]").count(),
            1,
            "exactly one group (group 0) may land at the LSB `:0]`"
        );
        let last_extract = pp
            .rfind("& data[")
            .and_then(|i| pp[i..].find(']').map(|j| &pp[i + "& data[".len()..i + j]))
            .expect("a group extract exists");
        assert_eq!(
            last_extract,
            format!("{}:0", group - 1),
            "the last concatenation term must be the LSB group `data[{}:0]`",
            group - 1
        );
    }
}

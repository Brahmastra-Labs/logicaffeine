use super::*;

fn k(id: &str) -> Kernel {
    kernel(id).unwrap_or_else(|| panic!("kernel {id} is in the catalog"))
}

#[test]
fn the_stimulus_generator_is_xorshift64_13_7_17() {
    let mut x = SEED;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    assert_eq!(next_state(SEED), x);
    assert_ne!(next_state(SEED), SEED);
    let mut s = SEED;
    for _ in 0..10_000 {
        s = next_state(s);
        assert_ne!(s, 0, "xorshift never reaches the zero state from a nonzero seed");
    }
}

#[test]
fn lane_zero_is_the_state_and_lanes_differ() {
    assert_eq!(lane_word(SEED, 0), SEED);
    let lanes: Vec<u64> = (0..64).map(|i| lane_word(SEED, i)).collect();
    for i in 0..lanes.len() {
        for j in i + 1..lanes.len() {
            assert_ne!(lanes[i], lanes[j], "lanes {i} and {j} collide");
        }
    }
}

/// The integer a chunk of `bits` bits holds, read as the field reads it.
fn as_int(v: u64, bits: u32, signed: bool) -> i128 {
    let v = if bits >= 64 { v } else { v & ((1u64 << bits) - 1) };
    if signed && bits < 64 && (v >> (bits - 1)) & 1 == 1 {
        v as i128 - (1i128 << bits)
    } else if signed && bits == 64 {
        v as i64 as i128
    } else {
        v as i128
    }
}

#[test]
fn a_field_value_stays_inside_its_declared_domain_in_both_modes() {
    for kernel in kernels() {
        for mode in [Mode::Shaped, Mode::Edge] {
            let stim = stimulus(&kernel, mode, 2_000);
            for (op, words) in stim.iter().enumerate() {
                for (p, port) in kernel.inputs.iter().enumerate() {
                    for (c, field) in port.fields.iter().enumerate() {
                        let chunk_bits = port.bits.saturating_sub(64 * c as u32).min(64);
                        let (lo, hi) = field.range();
                        let values: Vec<i128> = if field.lanes == 1 {
                            vec![as_int(words[p][c], chunk_bits, field.signed || field.base < 0)]
                        } else {
                            (0..field.lanes)
                                .map(|j| as_int(words[p][c] >> (j * field.width), field.width, field.signed))
                                .collect()
                        };
                        for v in values {
                            assert!(
                                (lo..=hi).contains(&v),
                                "{} {} op {op} chunk {c}: {v} outside [{lo}, {hi}] ({mode:?})",
                                kernel.id,
                                port.name
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_multi_lane_field_packs_sub_lanes_and_has_no_offset() {
    for kernel in kernels() {
        for port in &kernel.inputs {
            for field in &port.fields {
                assert!(field.lanes >= 1 && field.lanes * field.width <= 64, "{} {}", kernel.id, port.name);
                if field.lanes > 1 {
                    assert_eq!(field.base, 0, "{} {}: packed sub-lanes carry no offset", kernel.id, port.name);
                }
            }
        }
    }
}

#[test]
fn edge_mode_reaches_both_ends_of_every_shaped_field() {
    let kernel = k("K1");
    let stim = stimulus(&kernel, Mode::Edge, 512);
    for (p, port) in kernel.inputs.iter().enumerate() {
        let field = port.fields[0];
        let (lo, hi) = field.range();
        let seen: Vec<i128> = stim.iter().map(|w| w[p][0] as i64 as i128).collect();
        assert!(seen.contains(&lo), "{} never reached its minimum {lo}", port.name);
        assert!(seen.contains(&hi), "{} never reached its maximum {hi}", port.name);
        assert!(
            seen.iter().all(|v| *v == lo || *v == hi),
            "{} took a value off its edges",
            port.name
        );
    }
}

#[test]
fn stimulus_is_deterministic_and_the_modes_differ() {
    for kernel in kernels() {
        let a = stimulus(&kernel, Mode::Shaped, 64);
        let b = stimulus(&kernel, Mode::Shaped, 64);
        assert_eq!(a, b, "{}: stimulus is a function of the seed alone", kernel.id);
        let e = stimulus(&kernel, Mode::Edge, 64);
        assert_ne!(a, e, "{}: edge mode drives different values", kernel.id);
    }
}

#[test]
fn a_held_port_changes_only_at_its_hold_boundary() {
    let kernel = k("K3");
    let (wp, w) = kernel
        .inputs
        .iter()
        .enumerate()
        .find(|(_, p)| p.hold > 1)
        .expect("the GEMM tile holds its weights across a tile");
    let stim = stimulus(&kernel, Mode::Shaped, 64);
    for op in 1..stim.len() {
        let same = stim[op][wp] == stim[op - 1][wp];
        if op as u32 % w.hold == 0 {
            assert!(!same, "the weights reload at op {op}");
        } else {
            assert!(same, "the weights hold inside a tile (op {op})");
        }
    }
}

/// An independent transcription of the HFT decision, used only to cross-check the VM golden.
fn decide_ref(p: &[i64]) -> i64 {
    let (micro, fair, edge, base, max_order, inv, cap, ok) =
        (p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]);
    let (mut side, mut size) = (0, 0);
    if ok == 1 {
        if micro > fair + edge {
            side = 2;
            size = base;
        } else if micro < fair - edge {
            side = 1;
            size = base;
        }
    }
    if size > max_order {
        size = max_order;
    }
    let mut signed = 0;
    if side == 1 {
        signed = size;
    }
    if side == 2 {
        signed = -size;
    }
    let projected = inv + signed;
    if projected > cap || projected < -cap {
        signed = 0;
    }
    signed
}

#[test]
fn the_decide_golden_is_the_vm_and_agrees_with_an_independent_reference() {
    let kernel = k("K1");
    assert!(matches!(kernel.golden, Golden::Vm { .. }));
    let stim = stimulus(&kernel, Mode::Shaped, 300);
    let out = golden(&kernel, &stim).expect("the VM runs the decision");
    let mut decisions = std::collections::BTreeSet::new();
    for (op, (i, o)) in stim.iter().zip(&out).enumerate() {
        let args: Vec<i64> = i.iter().map(|w| w[0] as i64).collect();
        let want = decide_ref(&args);
        assert_eq!(o[0][0] as i64, want, "op {op}: {args:?}");
        decisions.insert(want.signum());
    }
    assert_eq!(
        decisions.len(),
        3,
        "the shaped domain makes buy, sell and flat decisions"
    );
}

#[test]
fn the_horner_golden_is_the_vm_polynomial() {
    let kernel = k("K2");
    let stim = stimulus(&kernel, Mode::Edge, 64);
    let out = golden(&kernel, &stim).expect("the VM runs the polynomial");
    for (i, o) in stim.iter().zip(&out) {
        let x = i[0][0] as i64;
        assert_eq!(o[0][0] as i64, ((2 * x + 5) * x - 3) * x + 100, "x = {x}");
    }
}

#[test]
fn the_shaped_domains_never_overflow_an_int() {
    let horner = k("K2");
    let (lo, hi) = horner.inputs[0].fields[0].range();
    for x in [lo, hi] {
        let x = x as i128;
        let v = ((2 * x + 5) * x - 3) * x + 100;
        assert!(v.abs() < (1i128 << 63), "Horner at {x} leaves Int");
    }
}

#[test]
fn the_butterfly_golden_is_the_montgomery_oracle() {
    let kernel = k("K5");
    let stim = stimulus(&kernel, Mode::Shaped, 256);
    let out = golden(&kernel, &stim).unwrap();
    for (i, o) in stim.iter().zip(&out) {
        let (a, b, z) = (i[0][0] as i16, i[1][0] as i16, i[2][0] as i16);
        let t = super::super::keccak_rtl::fqmul(z, b);
        assert_eq!(o[0][0] as u16, a.wrapping_add(t) as u16);
        assert_eq!(o[1][0] as u16, a.wrapping_sub(t) as u16);
    }
}

#[test]
fn the_keccak_golden_is_the_fips_202_permutation() {
    let kernel = k("K4");
    let zero = vec![vec![vec![0u64; 25]]];
    let out = golden(&kernel, &zero).unwrap();
    assert_eq!(
        out[0][0][0], 0xF1258F7940E1DDE7,
        "Keccak-f[1600] of the zero state, lane 0 (FIPS 202 known answer)"
    );
}

#[test]
fn the_gemm_golden_is_the_int8_matrix_product() {
    let kernel = k("K3");
    let stim = stimulus(&kernel, Mode::Edge, 16);
    let out = golden(&kernel, &stim).unwrap();
    let byte = |w: &[u64], i: usize| ((w[i / 8] >> (8 * (i % 8))) & 0xff) as u8 as i8 as i64;
    for (i, o) in stim.iter().zip(&out) {
        for c in 0..8 {
            let want: i64 = (0..8).map(|r| byte(&i[0], r) * byte(&i[1], r * 8 + c)).sum();
            let got = ((o[0][c / 2] >> (32 * (c % 2))) & 0xffff_ffff) as u32 as i32 as i64;
            assert_eq!(got, want, "column {c}");
        }
    }
}

#[test]
fn the_simt_golden_is_masked_axpb_with_a_sum() {
    let kernel = k("K6");
    let stim = stimulus(&kernel, Mode::Shaped, 64);
    let out = golden(&kernel, &stim).unwrap();
    for (i, o) in stim.iter().zip(&out) {
        let (a, b, mask) = (i[0][0] as i64, i[1][0] as i64, i[2][0]);
        let mut sum = 0i64;
        for lane in 0..4 {
            let x = i[3 + lane][0] as i64;
            let y = if mask >> lane & 1 == 1 { a * x + b } else { 0 };
            assert_eq!(o[lane][0] as i64, y, "lane {lane}");
            sum += y;
        }
        assert_eq!(o[4][0] as i64, sum);
    }
}

#[test]
fn the_misr_is_shift_with_parity_feedback_xor_the_fold() {
    assert_eq!(misr_step(0, 0), 0);
    assert_eq!(misr_step(0, 5), 5);
    assert_eq!(misr_step(1, 0), 2 | 1);
    assert_eq!(misr_step(0x8000_0000_0000_0000, 0), 1);
    assert_eq!(misr_step(3, 0), 6);
}

#[test]
fn the_signature_sees_every_output_bit() {
    let kernel = k("K4");
    let stim = stimulus(&kernel, Mode::Shaped, 4);
    let out = golden(&kernel, &stim).unwrap();
    let base = signature(&kernel, &out);
    for chunk in 0..25 {
        for bit in [0u32, 17, 63] {
            let mut flipped = out.clone();
            flipped[2][0][chunk] ^= 1u64 << bit;
            assert_ne!(
                signature(&kernel, &flipped),
                base,
                "a flip of lane {chunk} bit {bit} reaches the signature"
            );
        }
    }
}

fn ports(list: &[(&str, Dir, u32)]) -> Vec<DutPort> {
    list.iter()
        .map(|(n, d, b)| DutPort {
            name: n.to_string(),
            dir: *d,
            bits: *b,
        })
        .collect()
}

fn butterfly_candidate() -> Candidate {
    Candidate {
        kernel: "K5".into(),
        name: "k5_probe".into(),
        top: "probe".into(),
        clock: "clk".into(),
        reset: None,
        protocol: Protocol::Registered { latency: 2 },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("zeta", "z")],
        outputs: vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
        mems: vec![],
        batch: 1,
        ports: ports(&[
            ("clk", Dir::In, 1),
            ("a", Dir::In, 16),
            ("b", Dir::In, 16),
            ("z", Dir::In, 16),
            ("hi", Dir::Out, 16),
            ("lo", Dir::Out, 16),
        ]),
    }
}

#[test]
fn a_port_map_naming_a_missing_dut_port_is_refused() {
    let kernel = k("K5");
    let mut cand = butterfly_candidate();
    cand.ports = ports(&[
        ("clk", Dir::In, 1),
        ("a", Dir::In, 16),
        ("b", Dir::In, 16),
        ("hi", Dir::Out, 16),
        ("lo", Dir::Out, 16),
    ]);
    match check_ports(&kernel, &cand) {
        Err(QorError::PortMissing { port, .. }) => assert_eq!(port, "z"),
        other => panic!("expected PortMissing(z), got {other:?}"),
    }
}

#[test]
fn a_port_narrower_than_its_contract_is_refused() {
    let kernel = k("K5");
    let mut cand = butterfly_candidate();
    cand.ports = ports(&[
        ("clk", Dir::In, 1),
        ("a", Dir::In, 16),
        ("b", Dir::In, 16),
        ("z", Dir::In, 8),
        ("hi", Dir::Out, 16),
        ("lo", Dir::Out, 16),
    ]);
    assert!(matches!(
        check_ports(&kernel, &cand),
        Err(QorError::PortWidth { .. })
    ));
}

#[test]
fn a_port_driven_the_wrong_way_is_refused() {
    let kernel = k("K5");
    let mut cand = butterfly_candidate();
    cand.ports = ports(&[
        ("clk", Dir::In, 1),
        ("a", Dir::In, 16),
        ("b", Dir::In, 16),
        ("z", Dir::In, 16),
        ("hi", Dir::In, 16),
        ("lo", Dir::Out, 16),
    ]);
    assert!(matches!(
        check_ports(&kernel, &cand),
        Err(QorError::PortDirection { .. })
    ));
}

#[test]
fn every_contract_port_needs_a_mapping() {
    let kernel = k("K5");
    let mut cand = butterfly_candidate();
    cand.outputs.pop();
    cand.ports = ports(&[
        ("clk", Dir::In, 1),
        ("a", Dir::In, 16),
        ("b", Dir::In, 16),
        ("z", Dir::In, 16),
        ("hi", Dir::Out, 16),
        ("lo", Dir::Out, 16),
    ]);
    assert!(matches!(
        check_ports(&kernel, &cand),
        Err(QorError::Unmapped { .. })
    ));
}

#[test]
fn the_harness_keeps_the_dut_whole_and_exposes_only_clk_rst_sig() {
    let kernel = k("K5");
    let cand = butterfly_candidate();
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains("module qorbist_k5_probe(input clk, input rst, output [15:0] sig);"));
    assert!(v.contains("(* keep_hierarchy *) probe u_dut("));
    assert_eq!(v.matches("module ").count(), 1, "one module: the harness");
    assert!(v.contains(".z("), "the DUT port name, not the contract's");
}

#[test]
fn the_harness_names_its_stimulus_seed_and_the_same_lane_mixing_as_the_model() {
    let kernel = k("K1");
    let cand = logos_candidate("K1").expect("LOGOS builds K1").0;
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains(&format!("64'h{SEED:016x}")));
    assert!(v.contains(&format!("64'h{:016x}", 1u64.wrapping_mul(LANE_MIX))));
}

#[test]
fn the_testbench_reports_ops_and_signature_and_refuses_unknowns() {
    let cand = butterfly_candidate();
    let tb = emit_testbench(&cand, 100, 10_000, 1_000);
    assert!(tb.contains("QOR ops"));
    assert!(tb.contains("QOR X"));
    let run = parse_run("noise\nQOR first cycles 4\nQOR mid ops 50 cycles 112\nQOR ops 100 misr 00000000deadbeef cycles 212\n").unwrap();
    assert_eq!((run.ops, run.misr, run.cycles), (100, 0xdead_beef, 212));
    assert_eq!(run.first_cycles, Some(4));
    assert_eq!(run.mid, Some((50, 112)));
    assert_eq!(run.steady_cycles_per_op(), Some(2.0));
    assert!(tb.contains("QOR first cycles") && tb.contains("QOR mid ops"));
    assert!(matches!(
        parse_run("QOR X op 7\nQOR ops 7 misr xxxxxxxxxxxxxxxx\n"),
        Err(QorError::Unknown { op: 7 })
    ));
    assert!(matches!(parse_run("nothing"), Err(QorError::Testbench(_))));
}

#[test]
fn the_testbench_refuses_a_candidate_that_goes_idle_past_its_limit() {
    let tb = emit_testbench(&butterfly_candidate(), 100, 1_000_000, 777);
    assert!(tb.contains("idle >= 777"), "{tb}");
    assert!(tb.contains("QOR TIMEOUT idle 777 cycles ops"), "{tb}");
    assert!(tb.contains("if (u_h.ops != last_ops) begin last_ops = u_h.ops; idle = 0; end"), "a folded result restarts the idle count\n{tb}");
    assert!(matches!(parse_run("QOR first cycles 4\nQOR TIMEOUT idle 777 cycles ops 5 cycle 801\n"), Err(QorError::Testbench(m)) if m.contains("idle")));
}

#[test]
fn every_logos_candidate_maps_every_contract_port() {
    for id in ["K1", "K2", "K4", "K5"] {
        let kernel = k(id);
        let (cand, files) = logos_candidate(id).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!files.is_empty());
        assert_eq!(cand.inputs.len(), kernel.inputs.len(), "{id}");
        assert_eq!(cand.outputs.len(), kernel.outputs.len(), "{id}");
        assert!(
            files.iter().any(|(_, v)| v.contains(&format!("module {}(", cand.top))),
            "{id}: the candidate's top module is among its files"
        );
    }
}

fn seq_mem_ports(prefix: &str, addr_bits: u32, width: u32) -> Vec<(String, Dir, u32)> {
    vec![
        (format!("{prefix}_addr0"), Dir::Out, addr_bits),
        (format!("{prefix}_content_en"), Dir::Out, 1),
        (format!("{prefix}_write_en"), Dir::Out, 1),
        (format!("{prefix}_write_data"), Dir::Out, width),
        (format!("{prefix}_read_data"), Dir::In, width),
        (format!("{prefix}_done"), Dir::In, 1),
    ]
}

fn horner_seq_mem_candidate() -> Candidate {
    let mut list: Vec<(String, Dir, u32)> = vec![
        ("clk".into(), Dir::In, 1),
        ("reset".into(), Dir::In, 1),
        ("go".into(), Dir::In, 1),
        ("done".into(), Dir::Out, 1),
    ];
    list.extend(seq_mem_ports("x", 1, 64));
    list.extend(seq_mem_ports("y", 1, 64));
    Candidate {
        kernel: "K2".into(),
        name: "k2_mem_probe".into(),
        top: "main".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::GoDone { go: "go".into(), done: "done".into() },
        inputs: vec![],
        outputs: vec![],
        mems: vec![MemBind::seq_mem("x", "x", 1), MemBind::seq_mem("y", "y", 1)],
        batch: 1,
        ports: list.into_iter().map(|(name, dir, bits)| DutPort { name, dir, bits }).collect(),
    }
}

#[test]
fn a_seq_mem_binding_needs_its_whole_port_bundle() {
    let kernel = k("K2");
    let mut cand = horner_seq_mem_candidate();
    check_ports(&kernel, &cand).expect("the full bundle fits");
    cand.ports.retain(|p| p.name != "x_done");
    match check_ports(&kernel, &cand) {
        Err(QorError::PortMissing { port, .. }) => assert_eq!(port, "x_done"),
        other => panic!("expected PortMissing(x_done), got {other:?}"),
    }
}

#[test]
fn a_seq_mem_word_must_hold_the_contract_port() {
    let kernel = k("K2");
    let mut cand = horner_seq_mem_candidate();
    for p in cand.ports.iter_mut().filter(|p| p.name == "x_read_data") {
        p.bits = 32;
    }
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::PortWidth { .. })));
}

#[test]
fn a_contract_port_bound_both_ways_is_refused() {
    let kernel = k("K2");
    let mut cand = horner_seq_mem_candidate();
    cand.inputs.push(Bind::whole("x", "x_read_data"));
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Unmapped { .. })));
}

#[test]
fn the_harness_answers_a_seq_mem_with_one_cycle_latency() {
    let kernel = k("K2");
    let cand = horner_seq_mem_candidate();
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains(".x_read_data(m_x_read)"), "{v}");
    assert!(v.contains(".x_done(m_x_done)"));
    assert!(v.contains("m_x_done <= q_x_content_en;"), "a read or write completes one cycle later");
    assert!(v.contains("if (q_y_content_en && q_y_write_en)"), "the result memory captures writes");
}

fn gemm_mem2d_candidate() -> Candidate {
    let mut list: Vec<(String, Dir, u32)> = vec![
        ("clk".into(), Dir::In, 1),
        ("reset".into(), Dir::In, 1),
        ("go".into(), Dir::In, 1),
        ("done".into(), Dir::Out, 1),
    ];
    for (p, width) in [("a", 8), ("w", 8), ("o", 32)] {
        list.extend(seq_mem_ports(p, 4, width));
        list.push((format!("{p}_addr1"), Dir::Out, 4));
    }
    let rows = |p: &str| (0..8).map(|j| format!("{p}@{j}")).collect::<Vec<_>>();
    Candidate {
        kernel: "K3".into(),
        name: "k3_mem2d_probe".into(),
        top: "main".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::GoDone { go: "go".into(), done: "done".into() },
        inputs: vec![],
        outputs: vec![],
        mems: vec![
            MemBind::seq_mem_rows(&rows("a"), "a", 8).shaped(&[8, 8]),
            MemBind::seq_mem_rows(&["w@0".to_string()], "w", 64).shaped(&[8, 8]),
            MemBind::seq_mem_rows(&rows("o"), "o", 8).shaped(&[8, 8]),
        ],
        batch: 8,
        ports: list.into_iter().map(|(name, dir, bits)| DutPort { name, dir, bits }).collect(),
    }
}

#[test]
fn a_shaped_seq_mem_takes_one_address_port_per_dimension() {
    let kernel = k("K3");
    let mut cand = gemm_mem2d_candidate();
    check_ports(&kernel, &cand).expect("a row and a column address per memory");
    cand.ports.retain(|p| p.name != "w_addr1");
    match check_ports(&kernel, &cand) {
        Err(QorError::PortMissing { port, .. }) => assert_eq!(port, "w_addr1"),
        other => panic!("expected PortMissing(w_addr1), got {other:?}"),
    }
}

#[test]
fn a_shaped_address_port_must_reach_its_dimension() {
    let kernel = k("K3");
    let mut cand = gemm_mem2d_candidate();
    for p in cand.ports.iter_mut().filter(|p| p.name == "a_addr1") {
        p.bits = 2;
    }
    match check_ports(&kernel, &cand) {
        Err(QorError::PortWidth { port, need, .. }) => assert_eq!((port.as_str(), need), ("a_addr1", 3)),
        other => panic!("expected PortWidth(a_addr1), got {other:?}"),
    }
}

#[test]
fn a_shape_must_cover_the_memory_exactly() {
    let kernel = k("K3");
    let mut cand = gemm_mem2d_candidate();
    cand.mems[1] = cand.mems[1].clone().shaped(&[8, 4]);
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Protocol(_))), "32 cells cannot hold 64 words");
}

#[test]
fn a_block_ram_interface_takes_no_shape() {
    let kernel = k("K6");
    let mut cand = simt_bram_candidate();
    cand.mems[0] = cand.mems[0].clone().shaped(&[2, 2]);
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Protocol(_))), "a block RAM has one flat address per port");
}

#[test]
fn the_harness_addresses_a_shaped_memory_row_major() {
    let kernel = k("K3");
    let v = emit_harness(&kernel, &gemm_mem2d_candidate(), Mode::Shaped);
    for p in ["a", "w", "o"] {
        assert!(v.contains(&format!(".{p}_addr1(q_{p}_addr1)")), "{p}: the column address is connected\n{v}");
    }
    assert!(v.contains("(q_w_addr0 * 8 + q_w_addr1)"), "row times the column count plus column\n{v}");
}

#[test]
fn a_flat_memory_serialises_without_a_shape_and_a_shaped_one_round_trips() {
    let flat = MemBind::seq_mem("x", "x", 1);
    let json = serde_json::to_string(&flat).unwrap();
    assert!(!json.contains("dims"), "{json}");
    let back: MemBind = serde_json::from_str(r#"{"prefix":"x","kind":"SeqMem","contract":["x"],"words_per_port":1}"#).unwrap();
    assert_eq!(back, flat, "a manifest written before shapes reads as a flat memory");
    let shaped = MemBind::seq_mem_rows(&["w@0".to_string()], "w", 64).shaped(&[8, 8]);
    assert_eq!(serde_json::from_str::<MemBind>(&serde_json::to_string(&shaped).unwrap()).unwrap(), shaped);
}

fn simt_bram_candidate() -> Candidate {
    let mut list: Vec<(String, Dir, u32)> = vec![
        ("clock".into(), Dir::In, 1),
        ("reset".into(), Dir::In, 1),
        ("start_port".into(), Dir::In, 1),
        ("a".into(), Dir::In, 64),
        ("b".into(), Dir::In, 64),
        ("mask".into(), Dir::In, 8),
        ("done_port".into(), Dir::Out, 1),
        ("sum".into(), Dir::Out, 64),
    ];
    for i in 0..2 {
        list.push((format!("x_address{i}"), Dir::Out, 2));
        list.push((format!("x_ce{i}"), Dir::Out, 1));
        list.push((format!("x_q{i}"), Dir::In, 64));
        list.push((format!("y_address{i}"), Dir::Out, 2));
        list.push((format!("y_ce{i}"), Dir::Out, 1));
        list.push((format!("y_we{i}"), Dir::Out, 1));
        list.push((format!("y_d{i}"), Dir::Out, 64));
    }
    Candidate {
        kernel: "K6".into(),
        name: "k6_bram_probe".into(),
        top: "simt_arr".into(),
        clock: "clock".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::StartDone { start: "start_port".into(), done: "done_port".into() },
        inputs: vec![Bind::whole("a", "a"), Bind::whole("b", "b"), Bind::whole("mask", "mask")],
        outputs: vec![Bind::whole("sum", "sum")],
        mems: vec![
            MemBind::bram(&["x0", "x1", "x2", "x3"], "x", 2),
            MemBind::bram(&["y0", "y1", "y2", "y3"], "y", 2),
        ],
        batch: 1,
        ports: list.into_iter().map(|(name, dir, bits)| DutPort { name, dir, bits }).collect(),
    }
}

#[test]
fn a_bram_binding_gathers_contract_ports_into_one_array() {
    let kernel = k("K6");
    let cand = simt_bram_candidate();
    check_ports(&kernel, &cand).expect("four lanes in each array, two ports each");
    let mut short = cand.clone();
    short.mems[0] = MemBind::bram(&["x0", "x1", "x2"], "x", 2);
    assert!(
        matches!(check_ports(&kernel, &short), Err(QorError::Unmapped { .. })),
        "x3 left unbound is refused"
    );
}

#[test]
fn a_bram_port_needs_every_signal_of_its_direction() {
    let kernel = k("K6");
    let mut cand = simt_bram_candidate();
    cand.ports.retain(|p| p.name != "y_we1");
    match check_ports(&kernel, &cand) {
        Err(QorError::PortMissing { port, .. }) => assert_eq!(port, "y_we1"),
        other => panic!("expected PortMissing(y_we1), got {other:?}"),
    }
}

#[test]
fn the_harness_answers_every_bram_port_one_cycle_later() {
    let kernel = k("K6");
    let cand = simt_bram_candidate();
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    for i in 0..2 {
        assert!(v.contains(&format!(".x_q{i}(m_x_q{i})")), "{v}");
        assert!(v.contains(&format!("if (q_x_ce{i}) m_x_q{i} <=")));
        assert!(v.contains(&format!("if (q_y_ce{i} && q_y_we{i})")));
    }
}

#[test]
fn an_output_bind_reads_its_bits_from_an_offset_in_the_dut_port() {
    let kernel = k("K5");
    let mut cand = butterfly_candidate();
    cand.outputs = vec![
        Bind::from_dut("hi", "ret", 16, 16),
        Bind::from_dut("lo", "ret", 0, 16),
    ];
    cand.ports.retain(|p| p.name != "hi" && p.name != "lo");
    cand.ports.push(DutPort { name: "ret".into(), dir: Dir::Out, bits: 32 });
    check_ports(&kernel, &cand).expect("both halves of one 32-bit port");
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains("wire [15:0] r_hi = {q_ret[31:16]};"), "{v}");
    assert!(v.contains("wire [15:0] r_lo = {q_ret[15:0]};"));
    cand.outputs[0] = Bind::from_dut("hi", "ret", 20, 16);
    assert!(
        matches!(check_ports(&kernel, &cand), Err(QorError::PortWidth { .. })),
        "bits 35:20 do not exist in a 32-bit port"
    );
}

fn staggered_simt_candidate() -> Candidate {
    let mut list: Vec<(String, Dir, u32)> = vec![
        ("clock".into(), Dir::In, 1),
        ("reset".into(), Dir::In, 1),
        ("start_port".into(), Dir::In, 1),
        ("done_port".into(), Dir::Out, 1),
        ("a".into(), Dir::In, 64),
        ("b".into(), Dir::In, 64),
        ("mask".into(), Dir::In, 8),
    ];
    for x in ["x0", "x1", "x2", "x3"] {
        list.push((x.into(), Dir::In, 64));
    }
    for y in ["y0", "y1", "y2", "y3", "sum"] {
        list.push((y.into(), Dir::Out, 64));
        list.push((format!("{y}_vld"), Dir::Out, 1));
    }
    let outputs = ["y0", "y1", "y2", "y3", "sum"]
        .iter()
        .map(|y| Bind::whole(y, y).valid_on(&format!("{y}_vld")))
        .collect();
    Candidate {
        kernel: "K6".into(),
        name: "k6_valid_probe".into(),
        top: "simt_ports".into(),
        clock: "clock".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::Pipelined { start: "start_port".into(), done: "done_port".into(), ii: 1 },
        inputs: ["a", "b", "mask", "x0", "x1", "x2", "x3"].iter().map(|p| Bind::whole(p, p)).collect(),
        outputs,
        mems: vec![],
        batch: 1,
        ports: list.into_iter().map(|(name, dir, bits)| DutPort { name, dir, bits }).collect(),
    }
}

#[test]
fn a_valid_strobed_output_needs_its_strobe_port() {
    let kernel = k("K6");
    let mut cand = staggered_simt_candidate();
    check_ports(&kernel, &cand).expect("every output with its strobe");
    cand.ports.retain(|p| p.name != "y2_vld");
    match check_ports(&kernel, &cand) {
        Err(QorError::PortMissing { port, .. }) => assert_eq!(port, "y2_vld"),
        other => panic!("expected PortMissing(y2_vld), got {other:?}"),
    }
}

#[test]
fn strobed_and_plain_outputs_do_not_mix() {
    let kernel = k("K6");
    let mut cand = staggered_simt_candidate();
    cand.outputs[4] = Bind::whole("sum", "sum");
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Protocol(_))));
}

#[test]
fn strobed_outputs_queue_in_order_and_fold_when_every_port_has_one() {
    let kernel = k("K6");
    let cand = staggered_simt_candidate();
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains("if (q_y0_vld) begin"), "{v}");
    assert!(v.contains("wire capture = (n_y0 != 5'd0) && (n_y1 != 5'd0) && (n_y2 != 5'd0) && (n_y3 != 5'd0) && (n_sum != 5'd0);"));
    assert!(v.contains("ovf"), "an overflowing queue poisons the signature");
}

#[test]
fn an_output_memory_is_captured_with_its_last_write_applied() {
    let kernel = k("K6");
    let cand = simt_bram_candidate();
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains("mem_y_next = mem_y;"), "{v}");
    assert!(v.contains("if (q_y_ce1 && q_y_we1) mem_y_next[q_y_address1 * 64 +: 64] = q_y_d1[63:0];"));
    assert!(v.contains("wire [63:0] r_y3 = mem_y_next["));
}

fn register_file_candidate() -> Candidate {
    let list: Vec<(String, Dir, u32)> = vec![
        ("clk".into(), Dir::In, 1),
        ("rst".into(), Dir::In, 1),
        ("load".into(), Dir::In, 1),
        ("din".into(), Dir::In, 64),
        ("go".into(), Dir::In, 1),
        ("raddr".into(), Dir::In, 8),
        ("dout".into(), Dir::Out, 64),
        ("done".into(), Dir::Out, 1),
    ];
    let whole = |p: &str, bits: u32| RegSlot { contract: p.into(), lsb: 0, width: bits };
    Candidate {
        kernel: "K6".into(),
        name: "k6_regfile_probe".into(),
        top: "gpu_axpb".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::LoadGoRead {
            load: "load".into(),
            din: "din".into(),
            go: "go".into(),
            done: "done".into(),
            raddr: "raddr".into(),
            dout: "dout".into(),
            reset_each_op: true,
            load_slots: vec![whole("a", 64), whole("b", 64), whole("mask", 4), whole("x0", 64), whole("x1", 64), whole("x2", 64), whole("x3", 64)],
            read_slots: vec![whole("y0", 64), whole("y1", 64), whole("y2", 64), whole("y3", 64), whole("sum", 64)],
        },
        inputs: vec![],
        outputs: vec![],
        mems: vec![],
        batch: 1,
        ports: list.into_iter().map(|(name, dir, bits)| DutPort { name, dir, bits }).collect(),
    }
}

#[test]
fn a_register_file_protocol_covers_every_contract_port_with_its_slots() {
    let kernel = k("K6");
    let mut cand = register_file_candidate();
    check_ports(&kernel, &cand).expect("seven loads, five reads");
    if let Protocol::LoadGoRead { read_slots, .. } = &mut cand.protocol {
        read_slots.pop();
    }
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Unmapped { .. })), "sum never read");
}

#[test]
fn a_register_file_protocol_needs_its_control_ports_and_a_reset_to_reset_each_operation() {
    let kernel = k("K6");
    let mut cand = register_file_candidate();
    cand.ports.retain(|p| p.name != "raddr");
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::PortMissing { .. })));
    let mut cand = register_file_candidate();
    cand.reset = None;
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Protocol(_))));
}

#[test]
fn the_harness_loads_goes_waits_and_reads_in_slot_order() {
    let kernel = k("K6");
    let cand = register_file_candidate();
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains(".rst(rst || dut_rst)"), "{v}");
    assert!(v.contains("16'd2: c_din = {60'd0, p_mask[3:0]};"), "{v}");
    assert!(v.contains("16'd4: rd_sum[63:0] <= q_dout[63:0];"), "{v}");
    assert!(v.contains("wire capture = proto_capture;"));
}

#[test]
fn the_stimulus_jumps_ahead_by_a_matrix_power() {
    let mut s = SEED;
    for n in 0..20u32 {
        assert_eq!(advance(SEED, n), s, "{n} steps");
        s = next_state(s);
    }
    for n in [1u32, 2, 3, 8, 64] {
        let rows = jump_rows(n);
        for probe in [SEED, 1, 0x8000_0000_0000_0000, 0xdead_beef_cafe_f00d] {
            let by_rows = rows
                .iter()
                .enumerate()
                .fold(0u64, |acc, (bit, row)| acc | ((((probe & row).count_ones() & 1) as u64) << bit));
            assert_eq!(by_rows, advance(probe, n), "jump {n} from {probe:#x}");
        }
    }
}

#[test]
fn a_contract_reference_names_its_batch_slot() {
    assert_eq!(contract_ref("a"), ("a", 0));
    assert_eq!(contract_ref("a@3"), ("a", 3));
    assert_eq!(contract_ref("o@7"), ("o", 7));
}

fn batched_butterfly_candidate(batch: u32) -> Candidate {
    let mut list: Vec<(String, Dir, u32)> = vec![
        ("clk".into(), Dir::In, 1),
        ("rst".into(), Dir::In, 1),
        ("start".into(), Dir::In, 1),
        ("done".into(), Dir::Out, 1),
    ];
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for j in 0..batch {
        for p in ["a", "b", "zeta"] {
            list.push((format!("{p}{j}"), Dir::In, 16));
            inputs.push(Bind::whole(&format!("{p}@{j}"), &format!("{p}{j}")));
        }
        for p in ["hi", "lo"] {
            list.push((format!("{p}{j}"), Dir::Out, 16));
            outputs.push(Bind::whole(&format!("{p}@{j}"), &format!("{p}{j}")));
        }
    }
    Candidate {
        kernel: "K5".into(),
        name: format!("k5_batch{batch}_probe"),
        top: "probe".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: Protocol::StartDone { start: "start".into(), done: "done".into() },
        inputs,
        outputs,
        mems: vec![],
        batch,
        ports: list.into_iter().map(|(name, dir, bits)| DutPort { name, dir, bits }).collect(),
    }
}

#[test]
fn a_batched_candidate_binds_every_slot() {
    let kernel = k("K5");
    let mut cand = batched_butterfly_candidate(4);
    check_ports(&kernel, &cand).expect("four slots, each fully bound");
    cand.inputs.retain(|b| b.contract != "zeta@2");
    match check_ports(&kernel, &cand) {
        Err(QorError::Unmapped { contract, .. }) => assert!(contract.starts_with("zeta@2"), "{contract}"),
        other => panic!("expected Unmapped(zeta@2), got {other:?}"),
    }
    let mut stray = batched_butterfly_candidate(2);
    stray.inputs.push(Bind::whole("a@2", "a1"));
    assert!(matches!(check_ports(&kernel, &stray), Err(QorError::Unmapped { .. })), "slot 2 of a batch of 2");
}

#[test]
fn a_batched_harness_keeps_one_operand_register_per_slot_and_queues_results() {
    let kernel = k("K5");
    let cand = batched_butterfly_candidate(4);
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    for j in 0..4 {
        assert!(v.contains(&format!("reg  [15:0] p_a_{j};")), "{v}");
    }
    assert!(v.contains("s <= s_jump;"), "the state jumps a whole batch");
    assert!(v.contains("wire [63:0] fold_sel = (fp == 2'd0) ? fold_0 :"), "the fold pointer picks the next slot in order");
    assert!(v.contains("fp <= (fp == 2'd3) ? 2'd0 : fp + 2'd1;"), "one slot folds per cycle, in operation order");
}

fn ws_beats(latency: u32) -> Protocol {
    let mut drives = Vec::new();
    for t in 0..8u32 {
        drives.push(Drive { beat: t, dut: "w_load".into(), value: DriveValue::Const(1) });
        drives.push(Drive { beat: t, dut: "w_row".into(), value: DriveValue::Const(t as u64) });
        drives.push(Drive {
            beat: t,
            dut: "w_data".into(),
            value: DriveValue::Slice { contract: "w@0".into(), lsb: 64 * t, width: 64 },
        });
    }
    for t in 0..8u32 {
        for r in 0..8u32 {
            drives.push(Drive {
                beat: 8 + t,
                dut: format!("a{r}"),
                value: DriveValue::Slice { contract: format!("a@{t}"), lsb: 8 * r, width: 8 },
            });
        }
    }
    Protocol::Beats { period: 30, drives, captures: (0..8).map(|t| 8 + t + latency).collect() }
}

#[test]
fn a_beat_schedule_drives_only_ports_the_dut_has() {
    let kernel = k("K3");
    let mut ports: Vec<DutPort> = vec![
        DutPort { name: "clk".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "rst".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "w_load".into(), dir: Dir::In, bits: 1 },
        DutPort { name: "w_row".into(), dir: Dir::In, bits: 3 },
        DutPort { name: "w_data".into(), dir: Dir::In, bits: 64 },
    ];
    for r in 0..8 {
        ports.push(DutPort { name: format!("a{r}"), dir: Dir::In, bits: 64 });
        ports.push(DutPort { name: format!("o{r}"), dir: Dir::Out, bits: 32 });
    }
    let mut cand = Candidate {
        kernel: "K3".into(),
        name: "k3_ws_probe".into(),
        top: "ws".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "rst".into(), active_high: true }),
        protocol: ws_beats(15),
        inputs: vec![],
        outputs: (0..8u32)
            .flat_map(|t| (0..8u32).map(move |c| Bind::slice(&format!("o@{t}"), 32 * c, 32, &format!("o{c}"))))
            .collect(),
        mems: vec![],
        batch: 8,
        ports,
    };
    check_ports(&kernel, &cand).expect("the WS schedule fits the loadable array");
    if let Protocol::Beats { drives, .. } = &mut cand.protocol {
        drives.push(Drive { beat: 3, dut: "w_clear".into(), value: DriveValue::Const(1) });
    }
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::PortMissing { .. })));
}

/// A K5 butterfly on a two-beat call whose `b` and `zeta` latch with their own call and whose `a`
/// arrives one call later (`DriveValue::Previous`).
fn lagged_butterfly_candidate() -> Candidate {
    Candidate {
        kernel: "K5".into(),
        name: "k5_lag_probe".into(),
        top: "lag".into(),
        clock: "clk".into(),
        reset: None,
        protocol: Protocol::Beats {
            period: 2,
            drives: vec![
                Drive { beat: 0, dut: "load".into(), value: DriveValue::Const(1) },
                Drive { beat: 0, dut: "a_prev".into(), value: DriveValue::Previous { contract: "a".into(), lsb: 0, width: 16 } },
                Drive { beat: 0, dut: "b_now".into(), value: DriveValue::Slice { contract: "b".into(), lsb: 0, width: 16 } },
                Drive { beat: 0, dut: "zeta_now".into(), value: DriveValue::Slice { contract: "zeta".into(), lsb: 0, width: 16 } },
            ],
            captures: vec![3],
        },
        inputs: vec![],
        outputs: vec![Bind::whole("hi", "hi"), Bind::whole("lo", "lo")],
        mems: vec![],
        batch: 1,
        ports: ports(&[
            ("clk", Dir::In, 1),
            ("load", Dir::In, 1),
            ("a_prev", Dir::In, 16),
            ("b_now", Dir::In, 16),
            ("zeta_now", Dir::In, 16),
            ("hi", Dir::Out, 16),
            ("lo", Dir::Out, 16),
        ]),
    }
}

#[test]
fn a_previous_call_drive_binds_its_contract_bits() {
    let kernel = k("K5");
    let mut cand = lagged_butterfly_candidate();
    check_ports(&kernel, &cand).expect("a lagged `a` covers the contract port a");
    if let Protocol::Beats { drives, .. } = &mut cand.protocol {
        drives.retain(|d| d.dut != "a_prev");
    }
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Unmapped { .. })), "without it `a` is unbound");
    let mut cand = lagged_butterfly_candidate();
    if let Protocol::Beats { drives, .. } = &mut cand.protocol {
        drives[1].value = DriveValue::Previous { contract: "alpha".into(), lsb: 0, width: 16 };
    }
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Unmapped { .. })), "a lagged drive names a contract port");
}

#[test]
fn the_harness_keeps_the_previous_calls_operands_for_a_lagged_drive() {
    let v = emit_harness(&k("K5"), &lagged_butterfly_candidate(), Mode::Shaped);
    assert!(v.contains("reg  [15:0] pv_a;"), "one register per lagged port\n{v}");
    assert!(v.contains("pv_a <= 16'd0;"), "the call before the first is all zero\n{v}");
    assert!(v.contains("pv_a <= p_a;"), "it takes the call's operands as the next call begins\n{v}");
    assert!(v.contains("bd_a_prev = pv_a[15:0];"), "{v}");
    assert!(!v.contains("pv_b"), "only lagged ports are kept\n{v}");
}

#[test]
fn results_in_slot_order_need_one_capture_register_per_slot() {
    let captures: Vec<u32> = (0..8).map(|t| 25 + t).collect();
    assert_eq!(beat_capture_depths(8, &captures).unwrap(), vec![1; 8]);
    assert_eq!(beat_capture_depths(22, &(0..8).map(|t| 23 + t).collect::<Vec<_>>()).unwrap(), vec![1; 8]);
}

#[test]
fn results_that_leave_last_first_queue_until_the_fold_reaches_them() {
    // Slot j of call n arrives at 8n + 40 - j and is folded at 8n + 41 + j, after slot j of call n + 1
    // arrived (8n + 48 - j) whenever j >= 4.
    let captures: Vec<u32> = (0..8).map(|t| 40 - t).collect();
    assert_eq!(beat_capture_depths(8, &captures).unwrap(), vec![1, 1, 1, 1, 2, 2, 2, 2]);
    assert_eq!(beat_capture_depths(4, &[8, 7, 6, 5]).unwrap(), vec![1, 1, 2, 2]);
}

#[test]
fn a_schedule_that_outruns_one_fold_per_cycle_is_refused() {
    assert!(beat_capture_depths(4, &(0..8).map(|t| 10 + t).collect::<Vec<_>>()).is_err(), "eight results every four cycles");
}

#[test]
fn a_slot_with_a_deeper_queue_gets_one_and_the_others_keep_a_register() {
    let mut cand = gemm_reverse_probe();
    let v = emit_harness(&k("K3"), &cand, Mode::Shaped);
    assert!(v.contains("reg  [255:0] r_o_7_q_e [0:1];"), "slot 7 queues two results\n{v}");
    assert!(v.contains("wire [255:0] r_o_7_q = r_o_7_q_e[rp_7];"), "{v}");
    assert!(v.contains("reg  [255:0] r_o_0_q;"), "slot 0 keeps one register\n{v}");
    assert!(!v.contains("r_o_0_q_e"), "{v}");
    if let Protocol::Beats { period, .. } = &mut cand.protocol {
        *period = 4;
    }
    assert!(matches!(check_ports(&k("K3"), &cand), Err(QorError::Protocol(_))), "a period shorter than the batch is refused");
}

/// The LOGOS OS streaming array's binding with its rows captured last first (the probe never runs).
fn gemm_reverse_probe() -> Candidate {
    let (mut c, _) = gemm_os_candidate();
    if let Protocol::Beats { captures, .. } = &mut c.protocol {
        *captures = (0..8).map(|t| 40 - t).collect();
    }
    c
}

fn narrow_butterfly_candidate(dut_width: u32) -> Candidate {
    let mut c = butterfly_candidate();
    for p in c.ports.iter_mut().filter(|p| p.name == "hi") {
        p.bits = 12;
    }
    for b in c.outputs.iter_mut().filter(|b| b.dut == "hi") {
        *b = Bind::slice("hi", 0, 16, "hi").sign_extended(dut_width);
    }
    c
}

#[test]
fn a_narrow_signed_output_is_sign_extended_into_its_contract_field() {
    let kernel = k("K5");
    let cand = narrow_butterfly_candidate(12);
    check_ports(&kernel, &cand).expect("12 signed bits extended to the 16-bit field");
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains("{{4{q_hi[11]}}, q_hi[11:0]}"), "{v}");
    assert!(
        matches!(check_ports(&kernel, &narrow_butterfly_candidate(0)), Err(QorError::PortWidth { .. })),
        "without the extension a 12-bit port cannot carry 16 bits"
    );
    assert!(
        matches!(check_ports(&kernel, &narrow_butterfly_candidate(17)), Err(QorError::Protocol(_))),
        "the DUT cannot supply more bits than the bind covers"
    );
}

#[test]
fn a_bind_without_a_dut_width_reads_and_writes_as_before() {
    let b: Bind = serde_json::from_str(r#"{"contract":"hi","lsb":0,"width":16,"dut":"hi","dut_lsb":0}"#).unwrap();
    assert_eq!(b, Bind::slice("hi", 0, 16, "hi"));
    assert!(!serde_json::to_string(&b).unwrap().contains("dut_width"));
    let n = Bind::slice("hi", 0, 16, "hi").sign_extended(12);
    assert_eq!(serde_json::from_str::<Bind>(&serde_json::to_string(&n).unwrap()).unwrap(), n);
}

#[test]
fn the_gemm_arrays_come_with_matched_twenty_bit_accumulators() {
    let names: Vec<String> = logos_candidates("K3").unwrap().into_iter().map(|(c, _)| c.name).collect();
    assert_eq!(names, ["k3_logos_ws", "k3_logos_os", "k3_logos_ws_acc20", "k3_logos_os_acc20"]);
    let (_, files) = gemm_os_candidate_acc(20);
    assert!(files[0].1.contains("[19:0]"), "the accumulators are 20 bits");
}

/// A device candidate behind a one-port 64-byte-line bus and a kernel launch: eight K2 operands in
/// line 0x10, eight results written to line 0x20.
fn device_candidate() -> Candidate {
    let rows = |p: &str| (0..8).map(|j| format!("{p}@{j}")).collect::<Vec<_>>();
    Candidate {
        kernel: "K2".into(),
        name: "k2_dev_probe".into(),
        top: "dev".into(),
        clock: "clk".into(),
        reset: Some(Reset { port: "reset".into(), active_high: true }),
        protocol: Protocol::Launch {
            start: "start".into(),
            busy: "busy".into(),
            dcr: DcrBus {
                valid: "dcr_req_valid".into(),
                rw: "dcr_req_rw".into(),
                addr: "dcr_req_addr".into(),
                data: "dcr_req_data".into(),
                rsp_valid: "dcr_rsp_valid".into(),
            },
            setup: vec![DcrWrite { addr: 1, data: 0x10 }, DcrWrite { addr: 2, data: 0x20 }],
            flush: Some(0),
            bus: LineBus {
                ports: 1,
            line_bytes: 64,
            addr_bits: 16,
            tag_bits: 4,
            msb_first: true,
            prefix: "mem".into(),
            image: vec![],
            regions: vec![
                LineRegion { base_line: 0x10, contract: rows("x"), words_per_port: 1, stride_bits: 0 },
                LineRegion { base_line: 0x20, contract: rows("y"), words_per_port: 1, stride_bits: 0 },
            ],
            },
        },
        inputs: vec![],
        outputs: vec![],
        mems: vec![],
        batch: 8,
        ports: ports(&[
            ("clk", Dir::In, 1),
            ("reset", Dir::In, 1),
            ("mem_req_valid", Dir::Out, 1),
            ("mem_req_rw", Dir::Out, 1),
            ("mem_req_byteen", Dir::Out, 64),
            ("mem_req_addr", Dir::Out, 16),
            ("mem_req_data", Dir::Out, 512),
            ("mem_req_tag", Dir::Out, 4),
            ("mem_req_ready", Dir::In, 1),
            ("mem_rsp_valid", Dir::In, 1),
            ("mem_rsp_data", Dir::In, 512),
            ("mem_rsp_tag", Dir::In, 4),
            ("mem_rsp_ready", Dir::Out, 1),
            ("dcr_req_valid", Dir::In, 1),
            ("dcr_req_rw", Dir::In, 1),
            ("dcr_req_addr", Dir::In, 4),
            ("dcr_req_data", Dir::In, 32),
            ("dcr_rsp_valid", Dir::Out, 1),
            ("start", Dir::In, 1),
            ("busy", Dir::Out, 1),
        ]),
    }
}

fn bus_of(c: &mut Candidate) -> &mut LineBus {
    match &mut c.protocol {
        Protocol::Launch { bus, .. } => bus,
        _ => unreachable!("a device candidate"),
    }
}

#[test]
fn a_device_behind_a_line_bus_and_a_launch_fits_its_ports() {
    let kernel = k("K2");
    let cand = device_candidate();
    check_ports(&kernel, &cand).expect("the device binding fits");
    let mut no_tag = device_candidate();
    no_tag.ports.retain(|p| p.name != "mem_rsp_tag");
    assert!(matches!(check_ports(&kernel, &no_tag), Err(QorError::PortMissing { port, .. }) if port == "mem_rsp_tag"));
    let mut narrow = device_candidate();
    for p in narrow.ports.iter_mut().filter(|p| p.name == "mem_req_data") {
        p.bits = 256;
    }
    assert!(matches!(check_ports(&kernel, &narrow), Err(QorError::PortWidth { .. })), "a line is 512 bits per port");
    let mut no_busy = device_candidate();
    no_busy.ports.retain(|p| p.name != "busy");
    assert!(matches!(check_ports(&kernel, &no_busy), Err(QorError::PortMissing { .. })));
}

#[test]
fn every_contract_port_of_a_device_lives_in_a_region_once() {
    let kernel = k("K2");
    let mut cand = device_candidate();
    bus_of(&mut cand).regions.pop();
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Unmapped { .. })), "the results need a region");
    let mut cand = device_candidate();
    let first = bus_of(&mut cand).regions[0].clone();
    bus_of(&mut cand).regions.push(LineRegion { base_line: 0x30, ..first });
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Unmapped { .. })), "an operand in two regions");
    let mut cand = device_candidate();
    bus_of(&mut cand).regions[1].base_line = 0x10;
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Protocol(_))), "two regions on one line");
}

#[test]
fn a_program_image_line_is_one_whole_line() {
    let kernel = k("K2");
    let mut cand = device_candidate();
    bus_of(&mut cand).image.push(ImageLine { line: 0x40, hex: "00".repeat(64) });
    check_ports(&kernel, &cand).expect("64 bytes");
    bus_of(&mut cand).image.push(ImageLine { line: 0x41, hex: "00".repeat(32) });
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Protocol(_))), "half a line");
}

#[test]
fn the_harness_sets_the_device_up_launches_it_and_flushes_before_it_reads_the_results() {
    let mut cand = device_candidate();
    bus_of(&mut cand).image.push(ImageLine { line: 0x40, hex: "ab".repeat(64) });
    let v = emit_harness(&k("K2"), &cand, Mode::Shaped);
    assert!(v.contains(".mem_req_ready(1'b1)") || v.contains(".mem_req_ready(lb_ready)"), "the harness memory always accepts\n{v}");
    assert!(v.contains("16'h0040: lb_line = 512'h"), "the image line is served\n{v}");
    assert!(v.contains("lr_1_next"), "results are taken with the writes of the capture cycle\n{v}");
    assert!(v.contains("c_dcr_rw = 1'b0"), "the flush is a DCR read\n{v}");
}

#[test]
fn a_region_stride_gives_every_contract_port_a_whole_word() {
    let kernel = k("K5");
    let mut cand = device_candidate();
    cand.kernel = "K5".into();
    cand.batch = 2;
    let ops = |names: &[&str]| (0..2).flat_map(|j| names.iter().map(move |n| format!("{n}@{j}"))).collect::<Vec<_>>();
    bus_of(&mut cand).regions = vec![
        LineRegion { base_line: 0x10, contract: ops(&["a", "b", "zeta"]), words_per_port: 1, stride_bits: 64 },
        LineRegion { base_line: 0x20, contract: ops(&["hi", "lo"]), words_per_port: 1, stride_bits: 64 },
    ];
    check_ports(&kernel, &cand).expect("16-bit ports in 64-bit words");
    let v = emit_harness(&kernel, &cand, Mode::Shaped);
    assert!(v.contains("{{48{o_a_0[15]}}, o_a_0}"), "a signed operand fills its word by sign\n{v}");
    assert!(v.contains("lr_1_next[79:64]"), "lo@0 is the low 16 bits of the second word\n{v}");
    bus_of(&mut cand).regions[0].stride_bits = 8;
    assert!(matches!(check_ports(&kernel, &cand), Err(QorError::Protocol(_))), "a 16-bit port in an 8-bit stride");
}

/// The power block of a controller passes every port through by name, its clock and reset as `clk` and `rst`, an
/// active-low reset inverted, the design whole as `u_dut`.
#[test]
fn a_pass_wrapper_renames_the_clock_and_reset_and_passes_every_other_port() {
    let list = ports(&[
        ("sys_clk", Dir::In, 1),
        ("sys_rst_n", Dir::In, 1),
        ("cmd_valid", Dir::In, 1),
        ("cmd_payload_addr", Dir::In, 24),
        ("cmd_ready", Dir::Out, 1),
        ("rdata", Dir::Out, 64),
    ]);
    let v = emit_pass_wrapper("qorpwr_probe_p10", "ctrl", "sys_clk", Some(&Reset { port: "sys_rst_n".into(), active_high: false }), &list);
    assert!(v.contains("module qorpwr_probe_p10(\n  input clk,\n  input rst,\n"), "{v}");
    assert!(v.contains("(* keep_hierarchy *) ctrl u_dut("), "{v}");
    assert!(v.contains(".sys_clk(clk)") && v.contains(".sys_rst_n(!rst)"), "{v}");
    for (decl, conn) in [("input cmd_valid", ".cmd_valid(cmd_valid)"), ("input [23:0] cmd_payload_addr", ".cmd_payload_addr(cmd_payload_addr)"),
                         ("output cmd_ready", ".cmd_ready(cmd_ready)"), ("output [63:0] rdata", ".rdata(rdata)")] {
        assert!(v.contains(decl) && v.contains(conn), "{decl}\n{v}");
    }
    let without_reset = emit_pass_wrapper("qorpwr_probe_p10", "ctrl", "sys_clk", None, &list[..1]);
    assert!(without_reset.contains("module qorpwr_probe_p10(\n  input clk\n);"), "{without_reset}");
    let clash = ports(&[("ck", Dir::In, 1), ("clk", Dir::In, 1)]);
    assert!(std::panic::catch_unwind(|| emit_pass_wrapper("x", "ctrl", "ck", None, &clash)).is_err(), "a port named clk that is not the clock");
}

#[test]
fn a_fold_wrapper_drives_every_input_from_a_register_and_folds_every_output() {
    let ports = ports(&[
        ("clk", Dir::In, 1),
        ("rst", Dir::In, 1),
        ("req_valid", Dir::In, 1),
        ("req_addr", Dir::In, 26),
        ("wide", Dir::In, 100),
        ("cmd", Dir::Out, 5),
        ("data", Dir::Out, 70),
    ]);
    let v = emit_fold_wrapper("probe", "ctrl", "clk", Some(&Reset { port: "rst".into(), active_high: true }), &ports);
    assert!(v.contains("module qorsyn_probe(input clk, input rst, output [15:0] sig);"), "{v}");
    assert!(v.contains("(* keep_hierarchy *) ctrl u_dut("), "{v}");
    for p in ["req_valid", "req_addr", "wide"] {
        assert!(v.contains(&format!(".{p}(in_{p})")), "{p} comes from a register\n{v}");
        assert!(v.contains(&format!("in_{p} <=")), "{p} is refilled each cycle\n{v}");
    }
    assert!(v.contains(".clk(clk)") && v.contains(".rst(rst)"), "{v}");
    for p in ["cmd", "data"] {
        assert!(v.contains(&format!(".{p}(out_{p})")), "{v}");
    }
    assert!(v.contains("out_data[69:64]"), "every output bit reaches the fold\n{v}");
    assert!(v.contains("assign sig = fold[15:0];"), "{v}");
}

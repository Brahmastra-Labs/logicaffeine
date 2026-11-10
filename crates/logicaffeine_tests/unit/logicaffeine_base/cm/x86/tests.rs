use super::*;
use Field::*;

/// The field the decoder assigns each byte of `code`, in order.
fn fields(code: &[u8]) -> Vec<(Field, u8)> {
    let mut x = X86::new();
    code.iter()
        .map(|&b| {
            let f = (x.field(), x.index());
            x.push(b);
            f
        })
        .collect()
}

#[test]
fn a_prologue_a_call_and_an_epilogue() {
    // push ebp; mov ebp, esp; sub esp, 0x18; mov eax, [ebp+8]; call rel32; mov [esp+4], eax; leave; ret
    let code = [0x55, 0x89, 0xE5, 0x83, 0xEC, 0x18, 0x8B, 0x45, 0x08, 0xE8, 1, 2, 3, 4, 0x89, 0x44, 0x24, 0x04, 0xC9, 0xC3];
    let expected = [
        (Start, 0),
        (Start, 0),
        (ModRm, 0),
        (Start, 0),
        (ModRm, 0),
        (Immediate, 0),
        (Start, 0),
        (ModRm, 0),
        (Displacement, 0),
        (Start, 0),
        (Immediate, 0),
        (Immediate, 1),
        (Immediate, 2),
        (Immediate, 3),
        (Start, 0),
        (ModRm, 0),
        (Sib, 0),
        (Displacement, 0),
        (Start, 0),
        (Start, 0),
    ];
    assert_eq!(fields(&code), expected);
}

#[test]
fn the_operand_size_prefix_shrinks_immediates_and_the_address_size_prefix_shrinks_displacements() {
    // mov ax, 0x1234 (66 B8 iw); mov eax, [bp+0x10] (67 8B 46 db); mov eax, [0x1234] (67 8B 06 dw)
    let code = [0x66, 0xB8, 0x34, 0x12, 0x67, 0x8B, 0x46, 0x10, 0x67, 0x8B, 0x06, 0x34, 0x12, 0x90];
    let expected = [
        (Start, 0),
        (AfterPrefix, 0),
        (Immediate, 0),
        (Immediate, 1),
        (Start, 0),
        (AfterPrefix, 0),
        (ModRm, 0),
        (Displacement, 0),
        (Start, 0),
        (AfterPrefix, 0),
        (ModRm, 0),
        (Displacement, 0),
        (Displacement, 1),
        (Start, 0),
    ];
    assert_eq!(fields(&code), expected);
    // mov eax, imm32 with no prefix takes four bytes.
    assert_eq!(fields(&[0xB8, 1, 2, 3, 4, 0x90]).last(), Some(&(Start, 0)));
}

#[test]
fn the_two_and_three_byte_opcode_maps() {
    // jz rel32 (0F 84 cd); movzx eax, al (0F B6 C0); palignr xmm0, xmm1, 8 (66 0F 3A 0F C1 ib); pshufb (66 0F 38 00 C1)
    let code = [
        0x0F, 0x84, 1, 2, 3, 4, 0x0F, 0xB6, 0xC0, 0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x08, 0x66, 0x0F, 0x38, 0x00, 0xC1, 0x90,
    ];
    let expected = [
        (Start, 0),
        (Escape, 0),
        (Immediate, 0),
        (Immediate, 1),
        (Immediate, 2),
        (Immediate, 3),
        (Start, 0),
        (Escape, 0),
        (ModRm, 0),
        (Start, 0),
        (AfterPrefix, 0),
        (Escape, 0),
        (Escape3, 0),
        (ModRm, 0),
        (Immediate, 0),
        (Start, 0),
        (AfterPrefix, 0),
        (Escape, 0),
        (Escape3, 0),
        (ModRm, 0),
        (Start, 0),
    ];
    assert_eq!(fields(&code), expected);
}

#[test]
fn modrm_and_sib_decide_the_displacement() {
    let cases: [(&[u8], usize); 6] = [
        (&[0x8B, 0x04, 0x24], 0),             // mov eax, [esp]
        (&[0x8B, 0x05, 1, 2, 3, 4], 4),       // mov eax, [disp32]
        (&[0x8B, 0x44, 0x24, 0x08], 1),       // mov eax, [esp+disp8]
        (&[0x8B, 0x84, 0x24, 1, 2, 3, 4], 4), // mov eax, [esp+disp32]
        (&[0x8B, 0x04, 0x25, 1, 2, 3, 4], 4), // mov eax, [disp32] through a SIB with no base
        (&[0x8B, 0xC1], 0),                   // mov eax, ecx
    ];
    for (code, disp) in cases {
        let f = fields(code);
        assert_eq!(f.iter().filter(|(field, _)| *field == Displacement).count(), disp, "{code:02X?}");
        let mut x = X86::new();
        for &b in code {
            x.push(b);
        }
        assert_eq!(x.field(), Start, "{code:02X?} ends its instruction");
    }
}

#[test]
fn group_three_carries_an_immediate_only_for_test() {
    // test al, 0x7F (F6 C0 ib); neg eax (F7 D8); test ecx, imm32 (F7 C1 id); not byte [eax] (F6 10)
    let code = [0xF6, 0xC0, 0x7F, 0xF7, 0xD8, 0xF7, 0xC1, 1, 2, 3, 4, 0xF6, 0x10, 0x90];
    let imm: Vec<usize> = fields(&code).iter().enumerate().filter(|(_, f)| f.0 == Immediate).map(|(i, _)| i).collect();
    assert_eq!(imm, vec![2, 7, 8, 9, 10]);
    assert_eq!(fields(&code).last(), Some(&(Start, 0)));
}

#[test]
fn the_decoder_tracks_the_instructions_before_and_the_operand_so_far() {
    let mut x = X86::new();
    for &b in &[0x55, 0x89, 0xE5, 0xE8, 0x11, 0x22] {
        x.push(b);
    }
    assert_eq!(x.previous(), [0x89, 0x55]);
    assert_eq!((x.field(), x.index(), x.opcode()), (Immediate, 2, 0xE8));
    assert_eq!(x.operand(), 0x1122);
    assert_eq!(x.modrm(), 0x100, "a call has no ModRM byte");
    x.push(0x33);
    x.push(0x44);
    assert_eq!((x.field(), x.previous()[0]), (Start, 0xE8));
}

#[test]
fn any_byte_stream_decodes_without_panicking_and_every_field_occurs() {
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    let mut x = X86::new();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..1_000_000 {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        seen.insert(x.field());
        x.push((state >> 56) as u8);
    }
    for f in [Start, AfterPrefix, Escape, Escape3, ModRm, Sib, Displacement, Immediate] {
        assert!(seen.contains(&f), "{f:?} never occurred");
    }
}

// ---- What each instruction does ----------------------------------------------------------------

#[test]
fn a_prologue_a_call_and_an_epilogue_are_stack_move_arith_move_call_move_stack_return() {
    // push ebp; mov ebp, esp; sub esp, 0x18; mov eax, [ebp+8]; call rel32; mov [esp+4], eax; leave; ret
    let code = [0x55, 0x89, 0xE5, 0x83, 0xEC, 0x18, 0x8B, 0x45, 0x08, 0xE8, 1, 2, 3, 4, 0x89, 0x44, 0x24, 0x04, 0xC9, 0xC3];
    let mut x = X86::new();
    for &b in &code {
        x.push(b);
    }
    use Kind::*;
    let expected = [Stack, Move, Arith, Move, Call, Move, Stack, Return];
    let packed = expected.iter().fold(0u32, |k, &e| k << 4 | e as u32);
    assert_eq!(x.kinds(), packed, "the last eight kinds, latest lowest");
    assert_eq!(x.kind(), Other, "between instructions nothing is known of the next");
}

#[test]
fn a_group_opcode_takes_its_kind_from_the_modrm_reg_field() {
    use Kind::*;
    for (code, kind) in [
        (&[0x83, 0xC0, 0x01][..], Arith),
        (&[0x83, 0xE0, 0x01][..], Logic),
        (&[0x83, 0xF8, 0x01][..], Arith),
        (&[0x81, 0xC8, 1, 0, 0, 0][..], Logic),
        (&[0xF7, 0xC0, 1, 0, 0, 0][..], Logic),
        (&[0xF7, 0xD0][..], Logic),
        (&[0xF7, 0xD8][..], Arith),
        (&[0xF7, 0xF1][..], Arith),
        (&[0xFF, 0xC0][..], Arith),
        (&[0xFF, 0xD0][..], Call),
        (&[0xFF, 0x15, 1, 2, 3, 4][..], Call),
        (&[0xFF, 0xE0][..], Jump),
        (&[0xFF, 0x35, 1, 2, 3, 4][..], Stack),
        (&[0xC1, 0xE0, 0x04][..], Shift),
        (&[0xD1, 0xF8][..], Shift),
    ] {
        let mut x = X86::new();
        for &b in code {
            x.push(b);
        }
        assert_eq!(x.field(), Start, "{code:02X?} is one whole instruction");
        assert_eq!(x.kinds() & 0xF, kind as u32, "{code:02X?}");
    }
}

#[test]
fn the_kind_is_known_from_the_opcode_on_and_the_escape_maps_have_theirs() {
    use Kind::*;
    for (code, kind) in [
        (&[0x0F, 0x84, 1, 2, 3, 4][..], Branch),
        (&[0x75, 0x10][..], Branch),
        (&[0xEB, 0x10][..], Jump),
        (&[0xE8, 1, 2, 3, 4][..], Call),
        (&[0xC2, 0x08, 0x00][..], Return),
        (&[0x0F, 0xB6, 0xC0][..], Move),
        (&[0x0F, 0x44, 0xC1][..], Move),
        (&[0x0F, 0xAF, 0xC1][..], Arith),
        (&[0x0F, 0x94, 0xC0][..], Bit),
        (&[0x0F, 0xA3, 0xC8][..], Bit),
        (&[0x0F, 0xA4, 0xC8, 0x04][..], Shift),
        (&[0x0F, 0x10, 0x00][..], Vector),
        (&[0x66, 0x0F, 0xEF, 0xC0][..], Vector),
        (&[0x0F, 0x38, 0x00, 0xC1][..], Vector),
        (&[0x0F, 0x3A, 0x0F, 0xC1, 0x08][..], Vector),
        (&[0xD9, 0x45, 0x08][..], Float),
        (&[0xDD, 0xD8][..], Float),
        (&[0xF3, 0xA5][..], String),
        (&[0xAA][..], String),
        (&[0xCC][..], Interrupt),
        (&[0xCD, 0x80][..], Interrupt),
        (&[0x90][..], Misc),
        (&[0x99][..], Misc),
        (&[0xFC][..], Misc),
        (&[0x8D, 0x44, 0x24, 0x08][..], Move),
        (&[0x31, 0xC0][..], Logic),
        (&[0x85, 0xC0][..], Logic),
        (&[0x39, 0xC8][..], Arith),
        (&[0x40][..], Arith),
        (&[0x6A, 0x00][..], Stack),
        (&[0x0F, 0x31][..], Other),
    ] {
        let mut x = X86::new();
        for (i, &b) in code.iter().enumerate() {
            x.push(b);
            if i + 1 < code.len() && x.field() != Start {
                let started = !matches!(x.field(), AfterPrefix | Escape | Escape3);
                let group = matches!(code[0], 0x80..=0x83 | 0xC0 | 0xC1 | 0xD0..=0xD3 | 0xF6 | 0xF7 | 0xFE | 0xFF);
                if started && !group {
                    assert_eq!(x.kind(), kind, "{code:02X?}: known after its opcode");
                }
            }
        }
        assert_eq!(x.field(), Start, "{code:02X?} is one whole instruction");
        assert_eq!(x.kinds() & 0xF, kind as u32, "{code:02X?}");
    }
}

#[test]
fn every_opcode_has_a_kind_and_kinds_round_trip_through_their_bits() {
    for k in 0..16u8 {
        assert_eq!(Kind::from_bits(k) as u8, k);
    }
    let mut seen = [false; 16];
    for map in 0..4u16 {
        for op in 0..=255u8 {
            for reg in 0..8u8 {
                seen[kind_of(map << 8 | u16::from(op), Some(reg << 3)) as usize] = true;
            }
        }
    }
    assert!(seen.iter().all(|&s| s), "every kind names some opcode: {seen:?}");
}

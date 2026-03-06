	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	cmp	w0, #2
	b.ge	LBB0_2
; %bb.1:
	mov	w0, #1                          ; =0x1
	ret
LBB0_2:
	sub	sp, sp, #32
	stp	x29, x30, [sp, #16]             ; 16-byte Folded Spill
	add	x29, sp, #16
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	ldr	x0, [x1, #8]
	bl	_atoi
	cmp	w0, #1
	b.lt	LBB0_13
; %bb.3:
	mov	w8, #0                          ; =0x0
	mov	w9, #0                          ; =0x0
	scvtf	d0, w0
	movi	d1, #0000000000000000
	fmov	d2, #-1.00000000
	fmov	d3, #-1.50000000
	fmov	d4, #4.00000000
	fmov	d5, #1.00000000
	b	LBB0_5
LBB0_4:                                 ;   in Loop: Header=BB0_5 Depth=1
	fadd	d1, d1, d5
	add	w9, w9, #1
	cmp	w9, w0
	b.eq	LBB0_14
LBB0_5:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_8 Depth 2
                                        ;       Child Loop BB0_10 Depth 3
	mov	w10, #0                         ; =0x0
	fadd	d6, d1, d1
	fdiv	d6, d6, d0
	fadd	d6, d6, d2
	fmul	d7, d6, d6
	movi	d16, #0000000000000000
	b	LBB0_8
LBB0_6:                                 ;   in Loop: Header=BB0_8 Depth=2
	mov	w11, #0                         ; =0x0
LBB0_7:                                 ;   in Loop: Header=BB0_8 Depth=2
	add	w8, w8, w11
	fadd	d16, d16, d5
	add	w10, w10, #1
	cmp	w10, w0
	b.eq	LBB0_4
LBB0_8:                                 ;   Parent Loop BB0_5 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB0_10 Depth 3
	fadd	d17, d16, d16
	fdiv	d17, d17, d0
	fadd	d17, d17, d3
	fmadd	d18, d17, d17, d7
	fcmp	d18, d4
	b.gt	LBB0_6
; %bb.9:                                ;   in Loop: Header=BB0_8 Depth=2
	mov	w12, #0                         ; =0x0
	fmov	d18, d6
	fmov	d19, d17
LBB0_10:                                ;   Parent Loop BB0_5 Depth=1
                                        ;     Parent Loop BB0_8 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	mov	x11, x12
	cmp	w12, #49
	b.eq	LBB0_12
; %bb.11:                               ;   in Loop: Header=BB0_10 Depth=3
	fnmul	d20, d18, d18
	fmadd	d20, d19, d19, d20
	fadd	d19, d19, d19
	fadd	d20, d17, d20
	fmadd	d18, d19, d18, d6
	fmul	d19, d18, d18
	fmadd	d19, d20, d20, d19
	add	w12, w11, #1
	fcmp	d19, d4
	fmov	d19, d20
	b.le	LBB0_10
LBB0_12:                                ;   in Loop: Header=BB0_8 Depth=2
	cmp	w11, #48
	cset	w11, hi
	b	LBB0_7
LBB0_13:
	mov	w8, #0                          ; =0x0
LBB0_14:
	str	x8, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #32
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%d\n"

.subsections_via_symbols

	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	sub	sp, sp, #144
	stp	x28, x27, [sp, #48]             ; 16-byte Folded Spill
	stp	x26, x25, [sp, #64]             ; 16-byte Folded Spill
	stp	x24, x23, [sp, #80]             ; 16-byte Folded Spill
	stp	x22, x21, [sp, #96]             ; 16-byte Folded Spill
	stp	x20, x19, [sp, #112]            ; 16-byte Folded Spill
	stp	x29, x30, [sp, #128]            ; 16-byte Folded Spill
	add	x29, sp, #128
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_offset w25, -72
	.cfi_offset w26, -80
	.cfi_offset w27, -88
	.cfi_offset w28, -96
Lloh0:
	adrp	x8, ___stack_chk_guard@GOTPAGE
Lloh1:
	ldr	x8, [x8, ___stack_chk_guard@GOTPAGEOFF]
Lloh2:
	ldr	x8, [x8]
	str	x8, [sp, #40]
	cmp	w0, #1
	b.gt	LBB0_2
; %bb.1:
Lloh3:
	adrp	x8, ___stderrp@GOTPAGE
Lloh4:
	ldr	x8, [x8, ___stderrp@GOTPAGEOFF]
Lloh5:
	ldr	x3, [x8]
Lloh6:
	adrp	x0, l_.str@PAGE
Lloh7:
	add	x0, x0, l_.str@PAGEOFF
	mov	w20, #1                         ; =0x1
	mov	w1, #19                         ; =0x13
	mov	w2, #1                          ; =0x1
	bl	_fwrite
	b	LBB0_29
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	x20, x0
	mov	w0, #16                         ; =0x10
	bl	_malloc
	cbz	x0, LBB0_15
; %bb.3:
	mov	x19, x0
	cmp	w20, #1
	b.lt	LBB0_16
; %bb.4:
	mov	w26, #0                         ; =0x0
	mov	x24, #0                         ; =0x0
	mov	w25, #1                         ; =0x1
	mov	w21, #16                        ; =0x10
Lloh8:
	adrp	x22, l_.str.1@PAGE
Lloh9:
	add	x22, x22, l_.str.1@PAGEOFF
LBB0_5:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_6 Depth 2
	str	x26, [sp]
	add	x0, sp, #8
	mov	w1, #32                         ; =0x20
	mov	x2, x22
	bl	_snprintf
                                        ; kill: def $w0 killed $w0 def $x0
	sxtw	x23, w0
	add	x27, x24, x23
LBB0_6:                                 ;   Parent Loop BB0_5 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	cmp	x27, x21
	b.lo	LBB0_8
; %bb.7:                                ;   in Loop: Header=BB0_6 Depth=2
	lsl	x21, x21, #1
	mov	x0, x19
	mov	x1, x21
	bl	_realloc
	mov	x19, x0
	cbnz	x0, LBB0_6
	b	LBB0_10
LBB0_8:                                 ;   in Loop: Header=BB0_5 Depth=1
	add	x0, x19, x24
	add	x1, sp, #8
	mov	x2, x23
	bl	_memcpy
	add	w26, w26, #1
	mov	x24, x27
	cmp	w26, w20
	cset	w25, lt
	b.ne	LBB0_5
; %bb.9:
	mov	w20, #0                         ; =0x0
	mov	x24, x27
	b	LBB0_11
LBB0_10:
	mov	w20, #1                         ; =0x1
LBB0_11:
	tbnz	w25, #0, LBB0_29
; %bb.12:
	cbz	x24, LBB0_16
; %bb.13:
	cmp	x24, #8
	b.hs	LBB0_17
; %bb.14:
	mov	x8, #0                          ; =0x0
	mov	w9, #0                          ; =0x0
	b	LBB0_26
LBB0_15:
	mov	w20, #1                         ; =0x1
	b	LBB0_29
LBB0_16:
	mov	w9, #0                          ; =0x0
	b	LBB0_28
LBB0_17:
	cmp	x24, #64
	b.hs	LBB0_19
; %bb.18:
	mov	w9, #0                          ; =0x0
	mov	x8, #0                          ; =0x0
	b	LBB0_23
LBB0_19:
	mov	x9, #0                          ; =0x0
	movi.2d	v0, #0000000000000000
	movi.16b	v1, #32
	and	x8, x24, #0xffffffffffffffc0
	movi.4s	v2, #1
	movi.2d	v4, #0000000000000000
	movi.2d	v3, #0000000000000000
	movi.2d	v5, #0000000000000000
	movi.2d	v6, #0000000000000000
	movi.2d	v20, #0000000000000000
	movi.2d	v17, #0000000000000000
	movi.2d	v23, #0000000000000000
	movi.2d	v18, #0000000000000000
	movi.2d	v7, #0000000000000000
	movi.2d	v24, #0000000000000000
	movi.2d	v21, #0000000000000000
	movi.2d	v16, #0000000000000000
	movi.2d	v22, #0000000000000000
	movi.2d	v19, #0000000000000000
	movi.2d	v25, #0000000000000000
LBB0_20:                                ; =>This Inner Loop Header: Depth=1
	add	x10, x19, x9
	ldp	q26, q27, [x10]
	cmeq.16b	v26, v26, v1
	ushll.8h	v28, v26, #0
	ushll2.8h	v26, v26, #0
	ushll2.4s	v29, v26, #0
	and.16b	v29, v29, v2
	add.4s	v5, v5, v29
	ushll.4s	v29, v28, #0
	and.16b	v29, v29, v2
	ushll2.4s	v28, v28, #0
	and.16b	v28, v28, v2
	ushll.4s	v26, v26, #0
	and.16b	v26, v26, v2
	cmeq.16b	v27, v27, v1
	add.4s	v3, v3, v26
	ushll2.8h	v26, v27, #0
	add.4s	v4, v4, v28
	ushll2.4s	v28, v26, #0
	and.16b	v28, v28, v2
	add.4s	v0, v0, v29
	add.4s	v23, v23, v28
	ldp	q28, q29, [x10, #32]
	ushll.8h	v27, v27, #0
	ushll.4s	v26, v26, #0
	and.16b	v26, v26, v2
	add.4s	v17, v17, v26
	ushll.4s	v26, v27, #0
	and.16b	v26, v26, v2
	ushll2.4s	v27, v27, #0
	and.16b	v27, v27, v2
	cmeq.16b	v28, v28, v1
	add.4s	v20, v20, v27
	ushll2.8h	v27, v28, #0
	add.4s	v6, v6, v26
	ushll2.4s	v26, v27, #0
	and.16b	v26, v26, v2
	add.4s	v21, v21, v26
	ushll.8h	v26, v28, #0
	ushll.4s	v27, v27, #0
	and.16b	v27, v27, v2
	add.4s	v24, v24, v27
	ushll.4s	v27, v26, #0
	and.16b	v27, v27, v2
	ushll2.4s	v26, v26, #0
	and.16b	v26, v26, v2
	cmeq.16b	v28, v29, v1
	add.4s	v7, v7, v26
	ushll2.8h	v26, v28, #0
	add.4s	v18, v18, v27
	ushll2.4s	v27, v26, #0
	and.16b	v27, v27, v2
	add.4s	v25, v25, v27
	ushll.8h	v27, v28, #0
	ushll.4s	v26, v26, #0
	and.16b	v26, v26, v2
	add.4s	v19, v19, v26
	ushll2.4s	v26, v27, #0
	and.16b	v26, v26, v2
	add.4s	v22, v22, v26
	ushll.4s	v26, v27, #0
	and.16b	v26, v26, v2
	add.4s	v16, v16, v26
	add	x9, x9, #64
	cmp	x8, x9
	b.ne	LBB0_20
; %bb.21:
	add.4s	v1, v20, v4
	add.4s	v2, v23, v5
	add.4s	v0, v6, v0
	add.4s	v3, v17, v3
	add.4s	v3, v24, v3
	add.4s	v0, v18, v0
	add.4s	v2, v21, v2
	add.4s	v1, v7, v1
	add.4s	v1, v22, v1
	add.4s	v2, v25, v2
	add.4s	v0, v16, v0
	add.4s	v3, v19, v3
	add.4s	v0, v0, v3
	add.4s	v1, v1, v2
	add.4s	v0, v0, v1
	addv.4s	s0, v0
	fmov	w9, s0
	cmp	x24, x8
	b.eq	LBB0_28
; %bb.22:
	tst	x24, #0x38
	b.eq	LBB0_26
LBB0_23:
	mov	x10, x8
	and	x8, x24, #0xfffffffffffffff8
	movi.2d	v0, #0000000000000000
	movi.2d	v1, #0000000000000000
	mov.s	v1[0], w9
	movi.8b	v2, #32
	movi.4s	v3, #1
LBB0_24:                                ; =>This Inner Loop Header: Depth=1
	ldr	d4, [x19, x10]
	cmeq.8b	v4, v4, v2
	ushll.8h	v4, v4, #0
	ushll.4s	v5, v4, #0
	and.16b	v5, v5, v3
	ushll2.4s	v4, v4, #0
	and.16b	v4, v4, v3
	add.4s	v0, v0, v4
	add.4s	v1, v1, v5
	add	x10, x10, #8
	cmp	x8, x10
	b.ne	LBB0_24
; %bb.25:
	add.4s	v0, v1, v0
	addv.4s	s0, v0
	fmov	w9, s0
	b	LBB0_27
LBB0_26:
	ldrb	w10, [x19, x8]
	cmp	w10, #32
	cinc	w9, w9, eq
	add	x8, x8, #1
LBB0_27:
	cmp	x24, x8
	b.ne	LBB0_26
LBB0_28:
	str	x9, [sp]
Lloh10:
	adrp	x0, l_.str.2@PAGE
Lloh11:
	add	x0, x0, l_.str.2@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	w20, #0                         ; =0x0
LBB0_29:
	ldr	x8, [sp, #40]
Lloh12:
	adrp	x9, ___stack_chk_guard@GOTPAGE
Lloh13:
	ldr	x9, [x9, ___stack_chk_guard@GOTPAGEOFF]
Lloh14:
	ldr	x9, [x9]
	cmp	x9, x8
	b.ne	LBB0_31
; %bb.30:
	mov	x0, x20
	ldp	x29, x30, [sp, #128]            ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #112]            ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #96]             ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #80]             ; 16-byte Folded Reload
	ldp	x26, x25, [sp, #64]             ; 16-byte Folded Reload
	ldp	x28, x27, [sp, #48]             ; 16-byte Folded Reload
	add	sp, sp, #144
	ret
LBB0_31:
	bl	___stack_chk_fail
	.loh AdrpLdrGotLdr	Lloh0, Lloh1, Lloh2
	.loh AdrpAdd	Lloh6, Lloh7
	.loh AdrpLdrGotLdr	Lloh3, Lloh4, Lloh5
	.loh AdrpAdd	Lloh8, Lloh9
	.loh AdrpAdd	Lloh10, Lloh11
	.loh AdrpLdrGotLdr	Lloh12, Lloh13, Lloh14
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"Usage: strings <n>\n"

l_.str.1:                               ; @.str.1
	.asciz	"%d "

l_.str.2:                               ; @.str.2
	.asciz	"%d\n"

.subsections_via_symbols

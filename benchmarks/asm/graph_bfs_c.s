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
	sub	sp, sp, #80
	stp	x24, x23, [sp, #16]             ; 16-byte Folded Spill
	stp	x22, x21, [sp, #32]             ; 16-byte Folded Spill
	stp	x20, x19, [sp, #48]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #64]             ; 16-byte Folded Spill
	add	x29, sp, #64
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	ldr	x0, [x1, #8]
	bl	_atol
	mov	x21, x0
	add	x8, x0, x0, lsl #2
	lsl	x0, x8, #3
	bl	_malloc
	mov	x19, x0
	mov	x0, x21
	mov	w1, #4                          ; =0x4
	bl	_calloc
	mov	x20, x0
	cmp	x21, #1
	b.lt	LBB0_27
; %bb.3:
	mov	w8, #0                          ; =0x0
	mov	x9, #0                          ; =0x0
	mov	w10, #7                         ; =0x7
	b	LBB0_5
LBB0_4:                                 ;   in Loop: Header=BB0_5 Depth=1
	add	x9, x9, #1
	add	x10, x10, #31
	add	w8, w8, #5
	cmp	x21, x9
	b.eq	LBB0_7
LBB0_5:                                 ; =>This Inner Loop Header: Depth=1
	sdiv	x11, x10, x21
	msub	x11, x11, x21, x10
	cmp	x9, x11
	b.eq	LBB0_4
; %bb.6:                                ;   in Loop: Header=BB0_5 Depth=1
	lsl	x12, x9, #2
	ldr	w13, [x20, x12]
	add	w14, w8, w13
	str	x11, [x19, w14, sxtw #3]
	add	w11, w13, #1
	str	w11, [x20, x12]
	b	LBB0_4
LBB0_7:
	cmp	x21, #1
	b.lt	LBB0_27
; %bb.8:
	mov	w8, #0                          ; =0x0
	mov	x9, #0                          ; =0x0
	mov	w10, #13                        ; =0xd
	b	LBB0_10
LBB0_9:                                 ;   in Loop: Header=BB0_10 Depth=1
	add	x9, x9, #1
	add	w8, w8, #5
	add	x10, x10, #37
	cmp	x21, x9
	b.eq	LBB0_12
LBB0_10:                                ; =>This Inner Loop Header: Depth=1
	sdiv	x11, x10, x21
	msub	x11, x11, x21, x10
	cmp	x9, x11
	b.eq	LBB0_9
; %bb.11:                               ;   in Loop: Header=BB0_10 Depth=1
	lsl	x12, x9, #2
	ldr	w13, [x20, x12]
	add	w14, w8, w13
	str	x11, [x19, w14, sxtw #3]
	add	w11, w13, #1
	str	w11, [x20, x12]
	b	LBB0_9
LBB0_12:
	cmp	x21, #1
	b.lt	LBB0_27
; %bb.13:
	mov	w8, #0                          ; =0x0
	mov	x9, #0                          ; =0x0
	mov	w10, #17                        ; =0x11
	b	LBB0_15
LBB0_14:                                ;   in Loop: Header=BB0_15 Depth=1
	add	x9, x9, #1
	add	w8, w8, #5
	add	x10, x10, #41
	cmp	x21, x9
	b.eq	LBB0_17
LBB0_15:                                ; =>This Inner Loop Header: Depth=1
	sdiv	x11, x10, x21
	msub	x11, x11, x21, x10
	cmp	x9, x11
	b.eq	LBB0_14
; %bb.16:                               ;   in Loop: Header=BB0_15 Depth=1
	lsl	x12, x9, #2
	ldr	w13, [x20, x12]
	add	w14, w8, w13
	str	x11, [x19, w14, sxtw #3]
	add	w11, w13, #1
	str	w11, [x20, x12]
	b	LBB0_14
LBB0_17:
	cmp	x21, #1
	b.lt	LBB0_27
; %bb.18:
	mov	w8, #0                          ; =0x0
	mov	x9, #0                          ; =0x0
	mov	w10, #23                        ; =0x17
	b	LBB0_20
LBB0_19:                                ;   in Loop: Header=BB0_20 Depth=1
	add	x9, x9, #1
	add	w8, w8, #5
	add	x10, x10, #43
	cmp	x21, x9
	b.eq	LBB0_22
LBB0_20:                                ; =>This Inner Loop Header: Depth=1
	sdiv	x11, x10, x21
	msub	x11, x11, x21, x10
	cmp	x9, x11
	b.eq	LBB0_19
; %bb.21:                               ;   in Loop: Header=BB0_20 Depth=1
	lsl	x12, x9, #2
	ldr	w13, [x20, x12]
	add	w14, w8, w13
	str	x11, [x19, w14, sxtw #3]
	add	w11, w13, #1
	str	w11, [x20, x12]
	b	LBB0_19
LBB0_22:
	cmp	x21, #1
	b.lt	LBB0_27
; %bb.23:
	mov	w8, #0                          ; =0x0
	mov	x9, #0                          ; =0x0
	mov	w10, #29                        ; =0x1d
	b	LBB0_25
LBB0_24:                                ;   in Loop: Header=BB0_25 Depth=1
	add	x9, x9, #1
	add	w8, w8, #5
	add	x10, x10, #47
	cmp	x21, x9
	b.eq	LBB0_27
LBB0_25:                                ; =>This Inner Loop Header: Depth=1
	sdiv	x11, x10, x21
	msub	x11, x11, x21, x10
	cmp	x9, x11
	b.eq	LBB0_24
; %bb.26:                               ;   in Loop: Header=BB0_25 Depth=1
	lsl	x12, x9, #2
	ldr	w13, [x20, x12]
	add	w14, w8, w13
	str	x11, [x19, w14, sxtw #3]
	add	w11, w13, #1
	str	w11, [x20, x12]
	b	LBB0_24
LBB0_27:
	lsl	x24, x21, #3
	mov	x0, x24
	bl	_malloc
	mov	x22, x0
	mov	x0, x24
	bl	_malloc
	mov	x23, x0
	mov	w1, #255                        ; =0xff
	mov	x2, x24
	bl	_memset
	mov	x8, #0                          ; =0x0
	str	xzr, [x22]
	str	xzr, [x23]
	mov	w9, #1                          ; =0x1
	mov	w10, #40                        ; =0x28
	b	LBB0_29
LBB0_28:                                ;   in Loop: Header=BB0_29 Depth=1
	add	x8, x8, #1
	cmp	x8, x9
	b.ge	LBB0_34
LBB0_29:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_32 Depth 2
	ldr	x11, [x22, x8, lsl #3]
	ldr	w12, [x20, x11, lsl #2]
	cmp	w12, #1
	b.lt	LBB0_28
; %bb.30:                               ;   in Loop: Header=BB0_29 Depth=1
	madd	x13, x11, x10, x19
	b	LBB0_32
LBB0_31:                                ;   in Loop: Header=BB0_32 Depth=2
	subs	x12, x12, #1
	b.eq	LBB0_28
LBB0_32:                                ;   Parent Loop BB0_29 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldr	x14, [x13], #8
	ldr	x15, [x23, x14, lsl #3]
	cmn	x15, #1
	b.ne	LBB0_31
; %bb.33:                               ;   in Loop: Header=BB0_32 Depth=2
	ldr	x15, [x23, x11, lsl #3]
	add	x15, x15, #1
	str	x15, [x23, x14, lsl #3]
	str	x14, [x22, x9, lsl #3]
	add	x9, x9, #1
	b	LBB0_31
LBB0_34:
	cmp	x21, #1
	b.lt	LBB0_37
; %bb.35:
	cmp	x21, #8
	b.hs	LBB0_38
; %bb.36:
	mov	x8, #0                          ; =0x0
	mov	x10, #0                         ; =0x0
	mov	x9, #0                          ; =0x0
	b	LBB0_41
LBB0_37:
	mov	x9, #0                          ; =0x0
	mov	x10, #0                         ; =0x0
	b	LBB0_43
LBB0_38:
	movi.2d	v0, #0000000000000000
	and	x8, x21, #0xfffffffffffffff8
	movi.2d	v4, #0xffffffffffffffff
	add	x9, x23, #32
	movi.2d	v1, #0000000000000000
	mov	x10, x8
	movi.2d	v2, #0000000000000000
	movi.2d	v3, #0000000000000000
	movi.2d	v5, #0000000000000000
	movi.2d	v6, #0000000000000000
	movi.2d	v7, #0000000000000000
	movi.2d	v16, #0000000000000000
LBB0_39:                                ; =>This Inner Loop Header: Depth=1
	ldp	q17, q18, [x9, #-32]
	ldp	q19, q20, [x9], #64
	cmgt.2d	v21, v17, v4
	cmgt.2d	v22, v18, v4
	cmgt.2d	v23, v19, v4
	cmgt.2d	v24, v20, v4
	sub.2d	v5, v5, v21
	sub.2d	v6, v6, v22
	sub.2d	v7, v7, v23
	sub.2d	v16, v16, v24
	and.16b	v17, v17, v21
	and.16b	v18, v18, v22
	and.16b	v19, v19, v23
	and.16b	v20, v20, v24
	add.2d	v0, v17, v0
	add.2d	v1, v18, v1
	add.2d	v2, v19, v2
	add.2d	v3, v20, v3
	subs	x10, x10, #8
	b.ne	LBB0_39
; %bb.40:
	add.2d	v4, v6, v5
	add.2d	v5, v16, v7
	add.2d	v4, v5, v4
	addp.2d	d4, v4
	fmov	x9, d4
	add.2d	v0, v1, v0
	add.2d	v0, v2, v0
	add.2d	v0, v3, v0
	addp.2d	d0, v0
	fmov	x10, d0
	cmp	x21, x8
	b.eq	LBB0_43
LBB0_41:
	sub	x11, x21, x8
	add	x8, x23, x8, lsl #3
LBB0_42:                                ; =>This Inner Loop Header: Depth=1
	ldr	x12, [x8], #8
	mvn	x13, x12
	add	x9, x9, x13, lsr #63
	bic	x12, x12, x12, asr #63
	add	x10, x12, x10
	subs	x11, x11, #1
	b.ne	LBB0_42
LBB0_43:
	stp	x9, x10, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	x0, x20
	bl	_free
	mov	x0, x22
	bl	_free
	mov	x0, x23
	bl	_free
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #64]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #48]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #32]             ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #80
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%ld %ld\n"

.subsections_via_symbols

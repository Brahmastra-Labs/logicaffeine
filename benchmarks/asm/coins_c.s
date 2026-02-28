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
	sub	sp, sp, #48
	stp	x20, x19, [sp, #16]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #32]             ; 16-byte Folded Spill
	add	x29, sp, #32
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	ldr	x0, [x1, #8]
	bl	_atol
	mov	x19, x0
	add	x0, x0, #1
	mov	w1, #8                          ; =0x8
	bl	_calloc
	mov	x20, x0
	mov	w8, #1                          ; =0x1
	str	x8, [x0]
	cmp	x19, #1
	b.lt	LBB0_35
; %bb.3:
	mov	x9, #0                          ; =0x0
	mov	w8, #51719                      ; =0xca07
	movk	w8, #15258, lsl #16
	mov	x10, x20
	ldr	x11, [x10], #8
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	lsl	x12, x9, #3
	ldr	x13, [x10, x12]
	add	x11, x11, x13
	sdiv	x13, x11, x8
	msub	x11, x13, x8, x11
	str	x11, [x10, x12]
	add	x9, x9, #1
	cmp	x19, x9
	b.ne	LBB0_4
; %bb.5:
	cmp	x19, #5
	b.lt	LBB0_35
; %bb.6:
	mov	x9, #0                          ; =0x0
	sub	x10, x19, #4
LBB0_7:                                 ; =>This Inner Loop Header: Depth=1
	add	x11, x20, x9, lsl #3
	ldr	x12, [x11, #40]
	ldr	x13, [x11]
	add	x12, x13, x12
	sdiv	x13, x12, x8
	msub	x12, x13, x8, x12
	str	x12, [x11, #40]
	add	x9, x9, #1
	cmp	x10, x9
	b.ne	LBB0_7
; %bb.8:
	cmp	x19, #10
	b.lt	LBB0_35
; %bb.9:
	sub	x10, x19, #9
	cmp	x10, #2
	b.hs	LBB0_11
; %bb.10:
	mov	w9, #10                         ; =0xa
	b	LBB0_14
LBB0_11:
	and	x11, x10, #0xfffffffffffffffe
	add	x9, x11, #10
	add	x12, x20, #80
	mov	x13, #36837                     ; =0x8fe5
	movk	x13, #4770, lsl #16
	movk	x13, #24369, lsl #32
	movk	x13, #35184, lsl #48
	mov	w14, #51719                     ; =0xca07
	movk	w14, #15258, lsl #16
	mov	x15, x11
LBB0_12:                                ; =>This Inner Loop Header: Depth=1
	ldr	q0, [x12]
	ldur	q1, [x12, #-80]
	add.2d	v0, v1, v0
	mov.d	x16, v0[1]
	smulh	x17, x16, x13
	add	x17, x17, x16
	asr	x0, x17, #29
	add	x17, x0, x17, lsr #63
	msub	x16, x17, x14, x16
	fmov	x17, d0
	smulh	x0, x17, x13
	add	x0, x0, x17
	asr	x1, x0, #29
	add	x0, x1, x0, lsr #63
	msub	x17, x0, x14, x17
	fmov	d0, x17
	mov.d	v0[1], x16
	str	q0, [x12], #16
	subs	x15, x15, #2
	b.ne	LBB0_12
; %bb.13:
	cmp	x10, x11
	b.eq	LBB0_16
LBB0_14:
	sub	x10, x19, x9
	add	x10, x10, #1
	add	x9, x20, x9, lsl #3
LBB0_15:                                ; =>This Inner Loop Header: Depth=1
	ldr	x11, [x9]
	ldur	x12, [x9, #-80]
	add	x11, x12, x11
	sdiv	x12, x11, x8
	msub	x11, x12, x8, x11
	str	x11, [x9], #8
	subs	x10, x10, #1
	b.ne	LBB0_15
LBB0_16:
	cmp	x19, #25
	b.lt	LBB0_35
; %bb.17:
	mov	x9, #0                          ; =0x0
	sub	x10, x19, #24
LBB0_18:                                ; =>This Inner Loop Header: Depth=1
	add	x11, x20, x9, lsl #3
	ldr	x12, [x11, #200]
	ldr	x13, [x11]
	add	x12, x13, x12
	sdiv	x13, x12, x8
	msub	x12, x13, x8, x12
	str	x12, [x11, #200]
	add	x9, x9, #1
	cmp	x10, x9
	b.ne	LBB0_18
; %bb.19:
	cmp	x19, #50
	b.lt	LBB0_35
; %bb.20:
	sub	x10, x19, #49
	cmp	x10, #2
	b.hs	LBB0_22
; %bb.21:
	mov	w9, #50                         ; =0x32
	b	LBB0_25
LBB0_22:
	and	x11, x10, #0xfffffffffffffffe
	add	x9, x11, #50
	mov	x12, #36837                     ; =0x8fe5
	movk	x12, #4770, lsl #16
	movk	x12, #24369, lsl #32
	movk	x12, #35184, lsl #48
	mov	w13, #51719                     ; =0xca07
	movk	w13, #15258, lsl #16
	mov	x14, x11
	mov	x15, x20
LBB0_23:                                ; =>This Inner Loop Header: Depth=1
	ldr	q0, [x15, #400]
	ldr	q1, [x15]
	add.2d	v0, v1, v0
	mov.d	x16, v0[1]
	smulh	x17, x16, x12
	add	x17, x17, x16
	asr	x0, x17, #29
	add	x17, x0, x17, lsr #63
	msub	x16, x17, x13, x16
	fmov	x17, d0
	smulh	x0, x17, x12
	add	x0, x0, x17
	asr	x1, x0, #29
	add	x0, x1, x0, lsr #63
	msub	x17, x0, x13, x17
	fmov	d0, x17
	mov.d	v0[1], x16
	str	q0, [x15, #400]
	add	x15, x15, #16
	subs	x14, x14, #2
	b.ne	LBB0_23
; %bb.24:
	cmp	x10, x11
	b.eq	LBB0_27
LBB0_25:
	sub	x10, x19, x9
	add	x10, x10, #1
	add	x9, x20, x9, lsl #3
	sub	x9, x9, #400
LBB0_26:                                ; =>This Inner Loop Header: Depth=1
	ldr	x11, [x9, #400]
	ldr	x12, [x9]
	add	x11, x12, x11
	sdiv	x12, x11, x8
	msub	x11, x12, x8, x11
	str	x11, [x9, #400]
	add	x9, x9, #8
	subs	x10, x10, #1
	b.ne	LBB0_26
LBB0_27:
	cmp	x19, #100
	b.lt	LBB0_35
; %bb.28:
	sub	x10, x19, #99
	cmp	x10, #2
	b.hs	LBB0_30
; %bb.29:
	mov	w9, #100                        ; =0x64
	b	LBB0_33
LBB0_30:
	and	x11, x10, #0xfffffffffffffffe
	add	x9, x11, #100
	mov	x12, #36837                     ; =0x8fe5
	movk	x12, #4770, lsl #16
	movk	x12, #24369, lsl #32
	movk	x12, #35184, lsl #48
	mov	w13, #51719                     ; =0xca07
	movk	w13, #15258, lsl #16
	mov	x14, x11
	mov	x15, x20
LBB0_31:                                ; =>This Inner Loop Header: Depth=1
	ldr	q0, [x15, #800]
	ldr	q1, [x15]
	add.2d	v0, v1, v0
	mov.d	x16, v0[1]
	smulh	x17, x16, x12
	add	x17, x17, x16
	asr	x0, x17, #29
	add	x17, x0, x17, lsr #63
	msub	x16, x17, x13, x16
	fmov	x17, d0
	smulh	x0, x17, x12
	add	x0, x0, x17
	asr	x1, x0, #29
	add	x0, x1, x0, lsr #63
	msub	x17, x0, x13, x17
	fmov	d0, x17
	mov.d	v0[1], x16
	str	q0, [x15, #800]
	add	x15, x15, #16
	subs	x14, x14, #2
	b.ne	LBB0_31
; %bb.32:
	cmp	x10, x11
	b.eq	LBB0_35
LBB0_33:
	sub	x10, x19, x9
	add	x10, x10, #1
	add	x9, x20, x9, lsl #3
	sub	x9, x9, #800
LBB0_34:                                ; =>This Inner Loop Header: Depth=1
	ldr	x11, [x9, #800]
	ldr	x12, [x9]
	add	x11, x12, x11
	sdiv	x12, x11, x8
	msub	x11, x12, x8, x11
	str	x11, [x9, #800]
	add	x9, x9, #8
	subs	x10, x10, #1
	b.ne	LBB0_34
LBB0_35:
	ldr	x8, [x20, x19, lsl #3]
	str	x8, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x20
	bl	_free
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #32]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #48
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%ld\n"

.subsections_via_symbols

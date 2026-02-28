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
	mov	x20, x0
	add	x0, x0, #6
	bl	_malloc
	mov	x19, x0
	cmp	x20, #1
	b.lt	LBB0_10
; %bb.3:
	mov	x9, #0                          ; =0x0
	mov	x8, #63439                      ; =0xf7cf
	movk	x8, #58195, lsl #16
	movk	x8, #39845, lsl #32
	movk	x8, #8388, lsl #48
	mov	w10, #1000                      ; =0x3e8
	mov	w11, #88                        ; =0x58
	mov	w12, #22616                     ; =0x5858
	movk	w12, #22616, lsl #16
	mov	x13, #7378697629483820646       ; =0x6666666666666666
	movk	x13, #26215
	b	LBB0_5
LBB0_4:                                 ;   in Loop: Header=BB0_5 Depth=1
	smulh	x14, x9, x13
	lsr	x15, x14, #63
	add	w14, w15, w14, lsr #1
	add	w14, w14, w14, lsl #2
	sub	w14, w9, w14
	add	w14, w14, #97
	strb	w14, [x19, x9]
	add	x9, x9, #1
	cmp	x9, x20
	b.ge	LBB0_8
LBB0_5:                                 ; =>This Inner Loop Header: Depth=1
	cmp	x9, #1
	b.lt	LBB0_4
; %bb.6:                                ;   in Loop: Header=BB0_5 Depth=1
	lsr	x14, x9, #3
	umulh	x14, x14, x8
	lsr	x14, x14, #4
	msub	x14, x14, x10, x9
	cmp	x14, #0
	add	x14, x9, #5
	ccmp	x14, x20, #0, eq
	b.gt	LBB0_4
; %bb.7:                                ;   in Loop: Header=BB0_5 Depth=1
	add	x9, x19, x9
	strb	w11, [x9, #4]
	str	w12, [x9]
	mov	x9, x14
	cmp	x9, x20
	b.lt	LBB0_5
LBB0_8:
	strb	wzr, [x19, x20]
	cmp	x20, #5
	b.ge	LBB0_11
; %bb.9:
	mov	x8, #0                          ; =0x0
	b	LBB0_19
LBB0_10:
	mov	x8, #0                          ; =0x0
	strb	wzr, [x19, x20]
	b	LBB0_19
LBB0_11:
	mov	x8, #0                          ; =0x0
	sub	x9, x20, #4
	add	x10, x19, #2
LBB0_12:                                ; =>This Inner Loop Header: Depth=1
	ldurb	w11, [x10, #-2]
	cmp	w11, #88
	b.ne	LBB0_17
; %bb.13:                               ;   in Loop: Header=BB0_12 Depth=1
	ldurb	w11, [x10, #-1]
	cmp	w11, #88
	b.ne	LBB0_17
; %bb.14:                               ;   in Loop: Header=BB0_12 Depth=1
	ldrb	w11, [x10]
	cmp	w11, #88
	b.ne	LBB0_17
; %bb.15:                               ;   in Loop: Header=BB0_12 Depth=1
	ldrb	w11, [x10, #1]
	cmp	w11, #88
	b.ne	LBB0_17
; %bb.16:                               ;   in Loop: Header=BB0_12 Depth=1
	ldrb	w11, [x10, #2]
	cmp	w11, #88
	cset	w11, eq
	b	LBB0_18
LBB0_17:                                ;   in Loop: Header=BB0_12 Depth=1
	mov	x11, #0                         ; =0x0
LBB0_18:                                ;   in Loop: Header=BB0_12 Depth=1
	add	x10, x10, #1
	add	x8, x8, x11
	subs	x9, x9, #1
	b.ne	LBB0_12
LBB0_19:
	str	x8, [sp]
Lloh0:
	adrp	x0, l_.str.1@PAGE
Lloh1:
	add	x0, x0, l_.str.1@PAGEOFF
	bl	_printf
	mov	x0, x19
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
	.asciz	"XXXXX"

l_.str.1:                               ; @.str.1
	.asciz	"%ld\n"

.subsections_via_symbols

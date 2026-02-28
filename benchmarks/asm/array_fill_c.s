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
	lsl	x0, x0, #3
	bl	_malloc
	mov	x19, x0
	cmp	x20, #1
	b.lt	LBB0_13
; %bb.3:
	mov	w8, #16960                      ; =0x4240
	movk	w8, #15, lsl #16
	cmp	x20, #4
	b.hs	LBB0_5
; %bb.4:
	mov	x9, #0                          ; =0x0
	b	LBB0_8
LBB0_5:
	and	x9, x20, #0xfffffffffffffffc
	add	x10, x19, #16
	mov	w11, #24                        ; =0x18
	mov	w12, #17                        ; =0x11
	mov	w13, #10                        ; =0xa
	mov	w14, #3                         ; =0x3
	mov	x15, x9
LBB0_6:                                 ; =>This Inner Loop Header: Depth=1
	udiv	x16, x11, x8
	msub	x16, x16, x8, x11
	udiv	x17, x12, x8
	msub	x17, x17, x8, x12
	udiv	x0, x13, x8
	msub	x0, x0, x8, x13
	udiv	x1, x14, x8
	msub	x1, x1, x8, x14
	stp	x1, x0, [x10, #-16]
	add	x11, x11, #28
	add	x12, x12, #28
	stp	x17, x16, [x10], #32
	add	x13, x13, #28
	add	x14, x14, #28
	subs	x15, x15, #4
	b.ne	LBB0_6
; %bb.7:
	cmp	x20, x9
	b.eq	LBB0_10
LBB0_8:
	sub	x10, x20, x9
	lsl	x12, x9, #3
	add	x11, x19, x12
	sub	x9, x12, x9
	add	x9, x9, #3
LBB0_9:                                 ; =>This Inner Loop Header: Depth=1
	udiv	x12, x9, x8
	msub	x12, x12, x8, x9
	str	x12, [x11], #8
	add	x9, x9, #7
	subs	x10, x10, #1
	b.ne	LBB0_9
LBB0_10:
	cmp	x20, #1
	b.lt	LBB0_13
; %bb.11:
	mov	x8, #0                          ; =0x0
	mov	x9, #36837                      ; =0x8fe5
	movk	x9, #4770, lsl #16
	movk	x9, #24369, lsl #32
	movk	x9, #35184, lsl #48
	mov	w10, #51719                     ; =0xca07
	movk	w10, #15258, lsl #16
	mov	x11, x19
LBB0_12:                                ; =>This Inner Loop Header: Depth=1
	ldr	x12, [x11], #8
	add	x8, x12, x8
	smulh	x12, x8, x9
	add	x12, x12, x8
	asr	x13, x12, #29
	add	x12, x13, x12, lsr #63
	msub	x8, x12, x10, x8
	subs	x20, x20, #1
	b.ne	LBB0_12
	b	LBB0_14
LBB0_13:
	mov	x8, #0                          ; =0x0
LBB0_14:
	str	x8, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
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
	.asciz	"%ld\n"

.subsections_via_symbols

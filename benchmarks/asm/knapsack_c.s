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
	add	x24, x0, x0, lsl #2
	lsl	x20, x0, #3
	mov	x0, x20
	bl	_malloc
	mov	x19, x0
	mov	x0, x20
	bl	_malloc
	mov	x20, x0
	cmp	x21, #1
	b.lt	LBB0_5
; %bb.3:
	mov	w8, #8                          ; =0x8
	mov	w9, #7                          ; =0x7
	mov	w10, #4                         ; =0x4
	mov	w11, #3                         ; =0x3
	mov	x12, #62915                     ; =0xf5c3
	movk	x12, #23592, lsl #16
	movk	x12, #49807, lsl #32
	movk	x12, #10485, lsl #48
	mov	w13, #100                       ; =0x64
	mov	x14, #55051                     ; =0xd70b
	movk	x14, #28835, lsl #16
	movk	x14, #2621, lsl #32
	movk	x14, #41943, lsl #48
	mov	w15, #50                        ; =0x32
	mov	x16, x19
	mov	x17, x20
	mov	x0, x21
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	lsr	x1, x9, #2
	umulh	x1, x1, x12
	lsr	x1, x1, #2
	msub	x1, x1, x13, x8
	lsr	x2, x11, #1
	umulh	x2, x2, x14
	lsr	x2, x2, #4
	msub	x2, x2, x15, x10
	str	x2, [x16], #8
	str	x1, [x17], #8
	add	x8, x8, #31
	add	x9, x9, #31
	add	x10, x10, #17
	add	x11, x11, #17
	subs	x0, x0, #1
	b.ne	LBB0_4
LBB0_5:
	add	x23, x24, #1
	mov	x0, x23
	mov	w1, #8                          ; =0x8
	bl	_calloc
	mov	x22, x0
	mov	x0, x23
	mov	w1, #8                          ; =0x8
	bl	_calloc
	cmp	x21, #1
	b.lt	LBB0_13
; %bb.6:
	mov	x8, #0                          ; =0x0
	bic	x9, x24, x24, asr #63
	add	x9, x9, #1
	b	LBB0_8
LBB0_7:                                 ;   in Loop: Header=BB0_8 Depth=1
	add	x8, x8, #1
	mov	x0, x23
	cmp	x8, x21
	b.eq	LBB0_14
LBB0_8:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_10 Depth 2
	mov	x10, #0                         ; =0x0
	mov	x23, x22
	mov	x22, x0
	b	LBB0_10
LBB0_9:                                 ;   in Loop: Header=BB0_10 Depth=2
	add	x10, x10, #1
	cmp	x9, x10
	b.eq	LBB0_7
LBB0_10:                                ;   Parent Loop BB0_8 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	lsl	x12, x10, #3
	ldr	x11, [x23, x12]
	str	x11, [x22, x12]
	ldr	x12, [x19, x8, lsl #3]
	cmp	x10, x12
	b.lt	LBB0_9
; %bb.11:                               ;   in Loop: Header=BB0_10 Depth=2
	sub	x12, x23, x12, lsl #3
	ldr	x12, [x12, x10, lsl #3]
	ldr	x13, [x20, x8, lsl #3]
	add	x12, x13, x12
	cmp	x12, x11
	b.le	LBB0_9
; %bb.12:                               ;   in Loop: Header=BB0_10 Depth=2
	str	x12, [x22, x10, lsl #3]
	b	LBB0_9
LBB0_13:
	mov	x23, x0
LBB0_14:
	ldr	x8, [x22, x24, lsl #3]
	str	x8, [sp]
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
	.asciz	"%ld\n"

.subsections_via_symbols

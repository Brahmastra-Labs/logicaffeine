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
	mov	w24, #51719                     ; =0xca07
	movk	w24, #15258, lsl #16
	ldr	x0, [x1, #8]
	bl	_atol
	mov	x23, x0
	mul	x22, x0, x0
	lsl	x20, x22, #3
	mov	x0, x20
	bl	_malloc
	mov	x19, x0
	mov	x0, x20
	bl	_malloc
	mov	x20, x0
	mov	x0, x22
	mov	w1, #8                          ; =0x8
	bl	_calloc
	mov	x21, x0
	cmp	x23, #1
	b.lt	LBB0_14
; %bb.3:
	mov	x9, #0                          ; =0x0
	mov	x10, #0                         ; =0x0
	lsl	x8, x23, #3
	mov	x11, #55051                     ; =0xd70b
	movk	x11, #28835, lsl #16
	movk	x11, #2621, lsl #32
	movk	x11, #41943, lsl #48
	mov	w12, #100                       ; =0x64
	mov	x13, x19
	mov	x14, x20
LBB0_4:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_5 Depth 2
	mov	x15, #0                         ; =0x0
	mov	x16, x10
LBB0_5:                                 ;   Parent Loop BB0_4 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	add	x17, x9, x15
	smulh	x0, x17, x11
	add	x0, x0, x17
	asr	x1, x0, #6
	add	x0, x1, x0, lsr #63
	msub	x17, x0, x12, x17
	lsl	x0, x15, #3
	str	x17, [x13, x0]
	smulh	x17, x16, x11
	add	x17, x17, x16
	asr	x1, x17, #6
	add	x17, x1, x17, lsr #63
	msub	x17, x17, x12, x16
	str	x17, [x14, x0]
	add	x15, x15, #1
	add	x16, x16, x23
	cmp	x23, x15
	b.ne	LBB0_5
; %bb.6:                                ;   in Loop: Header=BB0_4 Depth=1
	add	x10, x10, #1
	add	x14, x14, x8
	add	x13, x13, x8
	add	x9, x9, x23
	cmp	x10, x23
	b.ne	LBB0_4
; %bb.7:
	cmp	x23, #1
	b.lt	LBB0_14
; %bb.8:
	mov	x9, #0                          ; =0x0
	mov	x10, x21
LBB0_9:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_10 Depth 2
                                        ;       Child Loop BB0_11 Depth 3
	mov	x11, #0                         ; =0x0
	mul	x12, x9, x23
	mov	x13, x20
LBB0_10:                                ;   Parent Loop BB0_9 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB0_11 Depth 3
	mov	x14, #0                         ; =0x0
	add	x15, x11, x12
	ldr	x15, [x19, x15, lsl #3]
LBB0_11:                                ;   Parent Loop BB0_9 Depth=1
                                        ;     Parent Loop BB0_10 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	lsl	x16, x14, #3
	ldr	x17, [x10, x16]
	ldr	x0, [x13, x16]
	madd	x17, x0, x15, x17
	sdiv	x0, x17, x24
	msub	x17, x0, x24, x17
	str	x17, [x10, x16]
	add	x14, x14, #1
	cmp	x23, x14
	b.ne	LBB0_11
; %bb.12:                               ;   in Loop: Header=BB0_10 Depth=2
	add	x11, x11, #1
	add	x13, x13, x8
	cmp	x11, x23
	b.ne	LBB0_10
; %bb.13:                               ;   in Loop: Header=BB0_9 Depth=1
	add	x9, x9, #1
	add	x10, x10, x8
	cmp	x9, x23
	b.ne	LBB0_9
LBB0_14:
	mov	x8, #0                          ; =0x0
	cbz	x23, LBB0_17
; %bb.15:
	cmp	x22, #1
	csinc	x9, x22, xzr, hi
	mov	x10, x21
LBB0_16:                                ; =>This Inner Loop Header: Depth=1
	ldr	x11, [x10], #8
	add	x8, x11, x8
	sdiv	x11, x8, x24
	msub	x8, x11, x24, x8
	subs	x9, x9, #1
	b.ne	LBB0_16
LBB0_17:
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
	mov	x0, x21
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

	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_swap                           ; -- Begin function swap
	.p2align	2
_swap:                                  ; @swap
	.cfi_startproc
; %bb.0:
	ldr	x8, [x0]
	ldr	x9, [x1]
	str	x9, [x0]
	str	x8, [x1]
	ret
	.cfi_endproc
                                        ; -- End function
	.globl	_partition                      ; -- Begin function partition
	.p2align	2
_partition:                             ; @partition
	.cfi_startproc
; %bb.0:
	ldr	x8, [x0, x2, lsl #3]
	subs	x9, x2, x1
	b.le	LBB1_6
; %bb.1:
	add	x10, x0, x1, lsl #3
	b	LBB1_3
LBB1_2:                                 ;   in Loop: Header=BB1_3 Depth=1
	add	x10, x10, #8
	subs	x9, x9, #1
	b.eq	LBB1_5
LBB1_3:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x11, [x10]
	cmp	x11, x8
	b.gt	LBB1_2
; %bb.4:                                ;   in Loop: Header=BB1_3 Depth=1
	lsl	x12, x1, #3
	ldr	x13, [x0, x12]
	str	x11, [x0, x12]
	str	x13, [x10]
	add	x1, x1, #1
	b	LBB1_2
LBB1_5:
	ldr	x8, [x0, x2, lsl #3]
LBB1_6:
	lsl	x9, x1, #3
	ldr	x10, [x0, x9]
	str	x8, [x0, x9]
	str	x10, [x0, x2, lsl #3]
	mov	x0, x1
	ret
	.cfi_endproc
                                        ; -- End function
	.globl	_qs                             ; -- Begin function qs
	.p2align	2
_qs:                                    ; @qs
	.cfi_startproc
; %bb.0:
	cmp	x1, x2
	b.ge	LBB2_8
; %bb.1:
	stp	x22, x21, [sp, #-48]!           ; 16-byte Folded Spill
	stp	x20, x19, [sp, #16]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #32]             ; 16-byte Folded Spill
	add	x29, sp, #32
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	mov	x19, x2
	mov	x20, x0
	lsl	x21, x2, #3
	b	LBB2_3
LBB2_2:                                 ;   in Loop: Header=BB2_3 Depth=1
	ldr	x8, [x20, x21]
	lsl	x9, x22, #3
	ldr	x10, [x20, x9]
	str	x8, [x20, x9]
	str	x10, [x20, x21]
	sub	x2, x22, #1
	mov	x0, x20
	bl	_qs
	add	x1, x22, #1
	cmp	x1, x19
	b.ge	LBB2_7
LBB2_3:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB2_5 Depth 2
	ldr	x8, [x20, x19, lsl #3]
	mov	x9, x1
	mov	x22, x1
	b	LBB2_5
LBB2_4:                                 ;   in Loop: Header=BB2_5 Depth=2
	add	x9, x9, #1
	cmp	x19, x9
	b.eq	LBB2_2
LBB2_5:                                 ;   Parent Loop BB2_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldr	x10, [x20, x9, lsl #3]
	cmp	x10, x8
	b.gt	LBB2_4
; %bb.6:                                ;   in Loop: Header=BB2_5 Depth=2
	lsl	x11, x22, #3
	ldr	x12, [x20, x11]
	str	x10, [x20, x11]
	str	x12, [x20, x9, lsl #3]
	add	x22, x22, #1
	b	LBB2_4
LBB2_7:
	ldp	x29, x30, [sp, #32]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #16]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp], #48             ; 16-byte Folded Reload
LBB2_8:
	ret
	.cfi_endproc
                                        ; -- End function
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	cmp	w0, #2
	b.ge	LBB3_2
; %bb.1:
	mov	w0, #1                          ; =0x1
	ret
LBB3_2:
	sub	sp, sp, #80
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
	ldr	x0, [x1, #8]
	bl	_atol
	mov	x20, x0
	lsl	x0, x0, #3
	bl	_malloc
	mov	x19, x0
	subs	x21, x20, #1
	b.lt	LBB3_8
; %bb.3:
	mov	x8, #0                          ; =0x0
	mov	w9, #42                         ; =0x2a
	mov	w10, #20077                     ; =0x4e6d
	movk	w10, #16838, lsl #16
	mov	w11, #12345                     ; =0x3039
LBB3_4:                                 ; =>This Inner Loop Header: Depth=1
	madd	x12, x9, x10, x11
	and	x9, x12, #0x7fffffff
	ubfx	x12, x12, #16, #15
	str	x12, [x19, x8, lsl #3]
	add	x8, x8, #1
	cmp	x20, x8
	b.ne	LBB3_4
; %bb.5:
	subs	x2, x20, #1
	mov	x0, x19
	mov	x1, #0                          ; =0x0
	bl	_qs
	mov	x8, #0                          ; =0x0
	subs	x21, x20, #1
	b.lt	LBB3_9
; %bb.6:
	mov	x9, #36837                      ; =0x8fe5
	movk	x9, #4770, lsl #16
	movk	x9, #24369, lsl #32
	movk	x9, #35184, lsl #48
	mov	w10, #51719                     ; =0xca07
	movk	w10, #15258, lsl #16
	mov	x11, x19
LBB3_7:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x12, [x11], #8
	add	x8, x12, x8
	smulh	x12, x8, x9
	add	x12, x12, x8
	asr	x13, x12, #29
	add	x12, x13, x12, lsr #63
	msub	x8, x12, x10, x8
	subs	x20, x20, #1
	b.ne	LBB3_7
	b	LBB3_9
LBB3_8:
	mov	x0, x19
	mov	x1, #0                          ; =0x0
	mov	x2, x21
	bl	_qs
	mov	x8, #0                          ; =0x0
LBB3_9:
	ldr	x9, [x19]
	ldr	x10, [x19, x21, lsl #3]
	stp	x10, x8, [sp, #8]
	str	x9, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #64]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #48]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #32]             ; 16-byte Folded Reload
	add	sp, sp, #80
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%ld %ld %ld\n"

.subsections_via_symbols

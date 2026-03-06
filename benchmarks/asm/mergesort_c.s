	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_merge_sort                     ; -- Begin function merge_sort
	.p2align	2
_merge_sort:                            ; @merge_sort
	.cfi_startproc
; %bb.0:
	cmp	x1, #2
	b.ge	LBB0_2
; %bb.1:
	ret
LBB0_2:
	stp	x28, x27, [sp, #-96]!           ; 16-byte Folded Spill
	stp	x26, x25, [sp, #16]             ; 16-byte Folded Spill
	stp	x24, x23, [sp, #32]             ; 16-byte Folded Spill
	stp	x22, x21, [sp, #48]             ; 16-byte Folded Spill
	stp	x20, x19, [sp, #64]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #80]             ; 16-byte Folded Spill
	add	x29, sp, #80
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
	mov	x19, x1
	mov	x21, x0
	lsr	x23, x1, #1
	lsl	x25, x23, #3
	mov	x0, x25
	bl	_malloc
	mov	x20, x0
	sub	x24, x19, x23
	lsl	x26, x24, #3
	mov	x0, x26
	bl	_malloc
	mov	x22, x0
	mov	x0, x20
	mov	x1, x21
	mov	x2, x25
	bl	_memcpy
	add	x1, x21, x25
	mov	x0, x22
	mov	x2, x26
	bl	_memcpy
	mov	x0, x20
	mov	x1, x23
	bl	_merge_sort
	mov	x0, x22
	mov	x1, x24
	bl	_merge_sort
	cmp	x24, #1
	b.lt	LBB0_10
; %bb.3:
	mov	x27, #0                         ; =0x0
	mov	x26, #0                         ; =0x0
	mov	x28, #0                         ; =0x0
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x8, [x20, x28, lsl #3]
	ldr	x9, [x22, x26, lsl #3]
	cmp	x8, x9
	csel	x8, x8, x9, lt
	cinc	x28, x28, le
	cinc	x26, x26, gt
	str	x8, [x21, x27, lsl #3]
	add	x27, x27, #1
	cmp	x28, x23
	ccmp	x26, x24, #0, lo
	b.lt	LBB0_4
; %bb.5:
	cmp	x28, x23
	b.ge	LBB0_7
LBB0_6:
	add	x0, x21, x27, lsl #3
	lsl	x8, x28, #3
	add	x1, x20, x8
	sub	x2, x25, x8
	bl	_memcpy
	add	x8, x27, x23
	sub	x27, x8, x28
LBB0_7:
	cmp	x26, x24
	b.ge	LBB0_9
; %bb.8:
	add	x0, x21, x27, lsl #3
	add	x8, x26, x23
	sub	x8, x19, x8
	lsl	x2, x8, #3
	add	x1, x22, x26, lsl #3
	bl	_memcpy
LBB0_9:
	mov	x0, x20
	bl	_free
	mov	x0, x22
	ldp	x29, x30, [sp, #80]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #64]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #48]             ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #32]             ; 16-byte Folded Reload
	ldp	x26, x25, [sp, #16]             ; 16-byte Folded Reload
	ldp	x28, x27, [sp], #96             ; 16-byte Folded Reload
	b	_free
LBB0_10:
	mov	x28, #0                         ; =0x0
	mov	x26, #0                         ; =0x0
	mov	x27, #0                         ; =0x0
	cmp	x28, x23
	b.lt	LBB0_6
	b	LBB0_7
	.cfi_endproc
                                        ; -- End function
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	cmp	w0, #2
	b.ge	LBB1_2
; %bb.1:
	mov	w0, #1                          ; =0x1
	ret
LBB1_2:
	sub	sp, sp, #64
	stp	x20, x19, [sp, #32]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #48]             ; 16-byte Folded Spill
	add	x29, sp, #48
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
	b.lt	LBB1_8
; %bb.3:
	mov	x8, #0                          ; =0x0
	mov	w9, #42                         ; =0x2a
	mov	w10, #20077                     ; =0x4e6d
	movk	w10, #16838, lsl #16
	mov	w11, #12345                     ; =0x3039
LBB1_4:                                 ; =>This Inner Loop Header: Depth=1
	madd	x12, x9, x10, x11
	and	x9, x12, #0x7fffffff
	ubfx	x12, x12, #16, #15
	str	x12, [x19, x8, lsl #3]
	add	x8, x8, #1
	cmp	x20, x8
	b.ne	LBB1_4
; %bb.5:
	mov	x0, x19
	mov	x1, x20
	bl	_merge_sort
	cmp	x20, #1
	b.lt	LBB1_9
; %bb.6:
	mov	x8, #0                          ; =0x0
	mov	x9, #36837                      ; =0x8fe5
	movk	x9, #4770, lsl #16
	movk	x9, #24369, lsl #32
	movk	x9, #35184, lsl #48
	mov	w10, #51719                     ; =0xca07
	movk	w10, #15258, lsl #16
	mov	x11, x19
	mov	x12, x20
LBB1_7:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x13, [x11], #8
	add	x8, x13, x8
	smulh	x13, x8, x9
	add	x13, x13, x8
	asr	x14, x13, #29
	add	x13, x14, x13, lsr #63
	msub	x8, x13, x10, x8
	subs	x12, x12, #1
	b.ne	LBB1_7
	b	LBB1_10
LBB1_8:
	mov	x0, x19
	mov	x1, x20
	bl	_merge_sort
LBB1_9:
	mov	x8, #0                          ; =0x0
LBB1_10:
	ldr	x9, [x19]
	add	x10, x19, x20, lsl #3
	ldur	x10, [x10, #-8]
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
	ldp	x29, x30, [sp, #48]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #32]             ; 16-byte Folded Reload
	add	sp, sp, #64
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%ld %ld %ld\n"

.subsections_via_symbols

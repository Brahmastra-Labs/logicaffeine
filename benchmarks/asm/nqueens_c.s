	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_solve                          ; -- Begin function solve
	.p2align	2
_solve:                                 ; @solve
	.cfi_startproc
; %bb.0:
	stp	x26, x25, [sp, #-80]!           ; 16-byte Folded Spill
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
	.cfi_offset w25, -72
	.cfi_offset w26, -80
	cmp	w1, w0
	b.ne	LBB0_2
; %bb.1:
	mov	w23, #1                         ; =0x1
	b	LBB0_6
LBB0_2:
	mov	x19, x4
	mov	x20, x3
	mov	x21, x2
	mov	x22, x0
	mov	w8, #-1                         ; =0xffffffff
	lsl	w8, w8, w0
	orr	w9, w2, w3
	orr	w9, w9, w4
	orr	w8, w8, w9
	cmn	w8, #1
	b.eq	LBB0_5
; %bb.3:
	mov	w23, #0                         ; =0x0
	mvn	w25, w8
	add	w24, w1, #1
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	neg	w8, w25
	and	w8, w25, w8
	eor	w25, w8, w25
	orr	w2, w8, w21
	orr	w9, w8, w20
	lsl	w3, w9, #1
	orr	w8, w8, w19
	asr	w4, w8, #1
	mov	x0, x22
	mov	x1, x24
	bl	_solve
	add	w23, w0, w23
	cbnz	w25, LBB0_4
	b	LBB0_6
LBB0_5:
	mov	w23, #0                         ; =0x0
LBB0_6:
	mov	x0, x23
	ldp	x29, x30, [sp, #64]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #48]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #32]             ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #16]             ; 16-byte Folded Reload
	ldp	x26, x25, [sp], #80             ; 16-byte Folded Reload
	ret
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
	sub	sp, sp, #32
	stp	x29, x30, [sp, #16]             ; 16-byte Folded Spill
	add	x29, sp, #16
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	w1, #0                          ; =0x0
	mov	w2, #0                          ; =0x0
	mov	w3, #0                          ; =0x0
	mov	w4, #0                          ; =0x0
	bl	_solve
                                        ; kill: def $w0 killed $w0 def $x0
	str	x0, [sp]
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

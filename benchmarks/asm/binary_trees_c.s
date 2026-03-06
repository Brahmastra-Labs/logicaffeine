	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_make_check                     ; -- Begin function make_check
	.p2align	2
_make_check:                            ; @make_check
	.cfi_startproc
; %bb.0:
	cbz	w0, LBB0_2
; %bb.1:
	stp	x29, x30, [sp, #-16]!           ; 16-byte Folded Spill
	mov	x29, sp
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	sub	w0, w0, #1
	bl	_make_check
	mov	w8, #1                          ; =0x1
	orr	x0, x8, x0, lsl #1
	ldp	x29, x30, [sp], #16             ; 16-byte Folded Reload
	ret
LBB0_2:
	mov	w0, #1                          ; =0x1
	ret
	.cfi_endproc
                                        ; -- End function
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	sub	sp, sp, #112
	stp	x26, x25, [sp, #32]             ; 16-byte Folded Spill
	stp	x24, x23, [sp, #48]             ; 16-byte Folded Spill
	stp	x22, x21, [sp, #64]             ; 16-byte Folded Spill
	stp	x20, x19, [sp, #80]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #96]             ; 16-byte Folded Spill
	add	x29, sp, #96
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
	mov	w19, #1                         ; =0x1
	cmp	w0, #2
	b.lt	LBB1_7
; %bb.1:
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	w8, #6                          ; =0x6
	cmp	w0, #6
	csel	w20, w0, w8, gt
	add	w21, w20, #1
	mov	x0, x21
	bl	_make_check
	stp	x21, x0, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x20
	bl	_make_check
	mov	x21, x0
	mov	w22, #4                         ; =0x4
	mov	x24, x20
Lloh2:
	adrp	x23, l_.str.1@PAGE
Lloh3:
	add	x23, x23, l_.str.1@PAGEOFF
	b	LBB1_4
LBB1_2:                                 ;   in Loop: Header=BB1_4 Depth=1
	mov	x0, x22
	bl	_make_check
	cmp	w25, #1
	csinc	w8, w25, wzr, gt
	mul	x8, x0, x8
LBB1_3:                                 ;   in Loop: Header=BB1_4 Depth=1
	stp	x22, x8, [sp, #8]
	str	x25, [sp]
	mov	x0, x23
	bl	_printf
	add	w22, w22, #2
	sub	w24, w24, #2
	cmp	w22, w20
	b.hi	LBB1_6
LBB1_4:                                 ; =>This Inner Loop Header: Depth=1
	lsl	w25, w19, w24
	cmp	w24, #31
	b.ne	LBB1_2
; %bb.5:                                ;   in Loop: Header=BB1_4 Depth=1
	mov	x8, #0                          ; =0x0
	b	LBB1_3
LBB1_6:
	stp	x20, x21, [sp]
Lloh4:
	adrp	x0, l_.str.2@PAGE
Lloh5:
	add	x0, x0, l_.str.2@PAGEOFF
	bl	_printf
	mov	w19, #0                         ; =0x0
LBB1_7:
	mov	x0, x19
	ldp	x29, x30, [sp, #96]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #80]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #64]             ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #48]             ; 16-byte Folded Reload
	ldp	x26, x25, [sp, #32]             ; 16-byte Folded Reload
	add	sp, sp, #112
	ret
	.loh AdrpAdd	Lloh2, Lloh3
	.loh AdrpAdd	Lloh0, Lloh1
	.loh AdrpAdd	Lloh4, Lloh5
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"stretch tree of depth %d\t check: %ld\n"

l_.str.1:                               ; @.str.1
	.asciz	"%d\t trees of depth %d\t check: %ld\n"

l_.str.2:                               ; @.str.2
	.asciz	"long lived tree of depth %d\t check: %ld\n"

.subsections_via_symbols

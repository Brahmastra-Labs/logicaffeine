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
	sub	sp, sp, #32
	stp	x29, x30, [sp, #16]             ; 16-byte Folded Spill
	add	x29, sp, #16
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	ldr	x0, [x1, #8]
	bl	_atol
	cmp	x0, #1
	b.lt	LBB0_6
; %bb.3:
	mov	x8, #0                          ; =0x0
	movi	d0, #0000000000000000
	fmov	d1, #1.00000000
	fmov	d2, #2.00000000
	fmov	d3, #1.00000000
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	scvtf	d4, x8
	fmadd	d4, d4, d2, d1
	fdiv	d4, d3, d4
	fadd	d0, d0, d4
	fneg	d3, d3
	add	x8, x8, #1
	cmp	x0, x8
	b.ne	LBB0_4
; %bb.5:
	fmov	d1, #4.00000000
	fmul	d0, d0, d1
	b	LBB0_7
LBB0_6:
	movi	d0, #0000000000000000
LBB0_7:
	str	d0, [sp]
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
	.asciz	"%.15f\n"

.subsections_via_symbols

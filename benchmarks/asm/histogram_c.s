	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	stp	x28, x27, [sp, #-48]!           ; 16-byte Folded Spill
	stp	x20, x19, [sp, #16]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #32]             ; 16-byte Folded Spill
	add	x29, sp, #32
	mov	w9, #8032                       ; =0x1f60
Lloh0:
	adrp	x16, ___chkstk_darwin@GOTPAGE
Lloh1:
	ldr	x16, [x16, ___chkstk_darwin@GOTPAGEOFF]
	blr	x16
	sub	sp, sp, #1, lsl #12             ; =4096
	sub	sp, sp, #3936
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w27, -40
	.cfi_offset w28, -48
Lloh2:
	adrp	x8, ___stack_chk_guard@GOTPAGE
Lloh3:
	ldr	x8, [x8, ___stack_chk_guard@GOTPAGEOFF]
Lloh4:
	ldr	x8, [x8]
	stur	x8, [x29, #-40]
	cmp	w0, #2
	b.ge	LBB0_2
; %bb.1:
	mov	w0, #1                          ; =0x1
	b	LBB0_8
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atol
	mov	x19, x0
	add	x20, sp, #24
	add	x0, sp, #24
	mov	w1, #8000                       ; =0x1f40
	bl	_bzero
	cmp	x19, #1
	b.lt	LBB0_5
; %bb.3:
	mov	w8, #42                         ; =0x2a
	mov	w9, #20077                      ; =0x4e6d
	movk	w9, #16838, lsl #16
	mov	w10, #12345                     ; =0x3039
	mov	w11, #33555                     ; =0x8313
	mov	w12, #1000                      ; =0x3e8
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	madd	x13, x8, x9, x10
	and	x8, x13, #0x7fffffff
	ubfx	w13, w13, #16, #15
	mul	w14, w13, w11
	lsr	w14, w14, #25
	msub	w13, w14, w12, w13
	and	x13, x13, #0xffff
	lsl	x13, x13, #3
	ldr	x14, [x20, x13]
	add	x14, x14, #1
	str	x14, [x20, x13]
	subs	x19, x19, #1
	b.ne	LBB0_4
LBB0_5:
	mov	x11, #0                         ; =0x0
	mov	x8, #0                          ; =0x0
	mov	x9, #0                          ; =0x0
	mov	x10, #0                         ; =0x0
LBB0_6:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x12, [x20, x11, lsl #3]
	cmp	x12, #0
	cinc	x8, x8, gt
	cmp	x12, x10
	csel	x10, x12, x10, gt
	csel	x9, x11, x9, gt
	add	x11, x11, #1
	cmp	x11, #1000
	b.ne	LBB0_6
; %bb.7:
	stp	x9, x8, [sp, #8]
	str	x10, [sp]
Lloh5:
	adrp	x0, l_.str@PAGE
Lloh6:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	w0, #0                          ; =0x0
LBB0_8:
	ldur	x8, [x29, #-40]
Lloh7:
	adrp	x9, ___stack_chk_guard@GOTPAGE
Lloh8:
	ldr	x9, [x9, ___stack_chk_guard@GOTPAGEOFF]
Lloh9:
	ldr	x9, [x9]
	cmp	x9, x8
	b.ne	LBB0_10
; %bb.9:
	add	sp, sp, #1, lsl #12             ; =4096
	add	sp, sp, #3936
	ldp	x29, x30, [sp, #32]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #16]             ; 16-byte Folded Reload
	ldp	x28, x27, [sp], #48             ; 16-byte Folded Reload
	ret
LBB0_10:
	bl	___stack_chk_fail
	.loh AdrpLdrGotLdr	Lloh2, Lloh3, Lloh4
	.loh AdrpLdrGot	Lloh0, Lloh1
	.loh AdrpAdd	Lloh5, Lloh6
	.loh AdrpLdrGotLdr	Lloh7, Lloh8, Lloh9
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%ld %ld %ld\n"

.subsections_via_symbols

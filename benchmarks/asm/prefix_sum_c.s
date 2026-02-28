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
	subs	x8, x20, #1
	b.lt	LBB0_8
; %bb.3:
	mov	x9, #0                          ; =0x0
	mov	w10, #42                        ; =0x2a
	mov	w11, #20077                     ; =0x4e6d
	movk	w11, #16838, lsl #16
	mov	w12, #12345                     ; =0x3039
	mov	w13, #33555                     ; =0x8313
	mov	w14, #1000                      ; =0x3e8
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	madd	x15, x10, x11, x12
	and	x10, x15, #0x7fffffff
	ubfx	w15, w15, #16, #15
	mul	w16, w15, w13
	lsr	w16, w16, #25
	msub	w15, w16, w14, w15
	and	x15, x15, #0xffff
	str	x15, [x19, x9, lsl #3]
	add	x9, x9, #1
	cmp	x20, x9
	b.ne	LBB0_4
; %bb.5:
	cmp	x20, #2
	b.lt	LBB0_8
; %bb.6:
	mov	x9, x19
	ldr	x12, [x9], #8
	mov	x10, #36837                     ; =0x8fe5
	movk	x10, #4770, lsl #16
	movk	x10, #24369, lsl #32
	movk	x10, #35184, lsl #48
	mov	w11, #51719                     ; =0xca07
	movk	w11, #15258, lsl #16
LBB0_7:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x13, [x9]
	add	x12, x12, x13
	smulh	x13, x12, x10
	add	x13, x13, x12
	asr	x14, x13, #29
	add	x13, x14, x13, lsr #63
	msub	x12, x13, x11, x12
	str	x12, [x9], #8
	subs	x8, x8, #1
	b.ne	LBB0_7
LBB0_8:
	add	x8, x19, x20, lsl #3
	ldur	x8, [x8, #-8]
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

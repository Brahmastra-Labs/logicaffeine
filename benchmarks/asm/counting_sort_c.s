	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	stp	x22, x21, [sp, #-48]!           ; 16-byte Folded Spill
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
	.cfi_offset w21, -40
	.cfi_offset w22, -48
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
	b	LBB0_25
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atol
	mov	x20, x0
	lsl	x0, x0, #3
	bl	_malloc
	mov	x19, x0
	cmp	x20, #1
	b.lt	LBB0_8
; %bb.3:
	mov	w8, #42                         ; =0x2a
	mov	w9, #20077                      ; =0x4e6d
	movk	w9, #16838, lsl #16
	mov	w10, #12345                     ; =0x3039
	mov	w11, #33555                     ; =0x8313
	mov	w12, #1000                      ; =0x3e8
	mov	x13, x19
	mov	x14, x20
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	madd	w15, w8, w9, w10
	and	w8, w15, #0x7fffffff
	ubfx	w15, w15, #16, #15
	mul	w16, w15, w11
	lsr	w16, w16, #25
	msub	w15, w16, w12, w15
	and	x15, x15, #0xffff
	str	x15, [x13], #8
	subs	x14, x14, #1
	b.ne	LBB0_4
; %bb.5:
	add	x21, sp, #24
	add	x0, sp, #24
	mov	w1, #8000                       ; =0x1f40
	bl	_bzero
	cmp	x20, #1
	b.lt	LBB0_9
; %bb.6:
	mov	x8, x19
	mov	x9, x20
LBB0_7:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x10, [x8], #8
	lsl	x10, x10, #3
	ldr	x11, [x21, x10]
	add	x11, x11, #1
	str	x11, [x21, x10]
	subs	x9, x9, #1
	b.ne	LBB0_7
	b	LBB0_9
LBB0_8:
	add	x0, sp, #24
	mov	w1, #8000                       ; =0x1f40
	bl	_bzero
LBB0_9:
	mov	x8, #0                          ; =0x0
	mov	x12, #0                         ; =0x0
	add	x9, x19, #32
	add	x10, sp, #24
	b	LBB0_12
LBB0_10:                                ;   in Loop: Header=BB0_12 Depth=1
	mov	x12, x11
LBB0_11:                                ;   in Loop: Header=BB0_12 Depth=1
	add	x8, x8, #1
	cmp	x8, #1000
	b.eq	LBB0_20
LBB0_12:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_16 Depth 2
                                        ;     Child Loop BB0_19 Depth 2
	ldr	x13, [x10, x8, lsl #3]
	cmp	x13, #1
	b.lt	LBB0_11
; %bb.13:                               ;   in Loop: Header=BB0_12 Depth=1
	add	x11, x12, x13
	cmp	x13, #8
	b.hs	LBB0_15
; %bb.14:                               ;   in Loop: Header=BB0_12 Depth=1
	mov	x14, x12
	b	LBB0_18
LBB0_15:                                ;   in Loop: Header=BB0_12 Depth=1
	and	x15, x13, #0xfffffffffffffff8
	add	x14, x12, x15
	dup.2d	v0, x8
	add	x12, x9, x12, lsl #3
	mov	x16, x15
LBB0_16:                                ;   Parent Loop BB0_12 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	stp	q0, q0, [x12, #-32]
	stp	q0, q0, [x12], #64
	subs	x16, x16, #8
	b.ne	LBB0_16
; %bb.17:                               ;   in Loop: Header=BB0_12 Depth=1
	cmp	x13, x15
	b.eq	LBB0_10
LBB0_18:                                ;   in Loop: Header=BB0_12 Depth=1
	sub	x12, x11, x14
	add	x13, x19, x14, lsl #3
LBB0_19:                                ;   Parent Loop BB0_12 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	str	x8, [x13], #8
	subs	x12, x12, #1
	b.ne	LBB0_19
	b	LBB0_10
LBB0_20:
	cmp	x20, #1
	b.lt	LBB0_23
; %bb.21:
	mov	x8, #0                          ; =0x0
	mov	x9, #36837                      ; =0x8fe5
	movk	x9, #4770, lsl #16
	movk	x9, #24369, lsl #32
	movk	x9, #35184, lsl #48
	mov	w10, #51719                     ; =0xca07
	movk	w10, #15258, lsl #16
	mov	x11, x19
	mov	x12, x20
LBB0_22:                                ; =>This Inner Loop Header: Depth=1
	ldr	x13, [x11], #8
	add	x8, x13, x8
	smulh	x13, x8, x9
	add	x13, x13, x8
	asr	x14, x13, #29
	add	x13, x14, x13, lsr #63
	msub	x8, x13, x10, x8
	subs	x12, x12, #1
	b.ne	LBB0_22
	b	LBB0_24
LBB0_23:
	mov	x8, #0                          ; =0x0
LBB0_24:
	ldr	x9, [x19]
	add	x10, x19, x20, lsl #3
	ldur	x10, [x10, #-8]
	stp	x10, x8, [sp, #8]
	str	x9, [sp]
Lloh5:
	adrp	x0, l_.str@PAGE
Lloh6:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	w0, #0                          ; =0x0
LBB0_25:
	ldur	x8, [x29, #-40]
Lloh7:
	adrp	x9, ___stack_chk_guard@GOTPAGE
Lloh8:
	ldr	x9, [x9, ___stack_chk_guard@GOTPAGEOFF]
Lloh9:
	ldr	x9, [x9]
	cmp	x9, x8
	b.ne	LBB0_27
; %bb.26:
	add	sp, sp, #1, lsl #12             ; =4096
	add	sp, sp, #3936
	ldp	x29, x30, [sp, #32]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #16]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp], #48             ; 16-byte Folded Reload
	ret
LBB0_27:
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

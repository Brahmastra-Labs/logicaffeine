	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	sub	sp, sp, #96
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
	cmp	w0, #1
	b.gt	LBB0_2
; %bb.1:
Lloh0:
	adrp	x8, ___stderrp@GOTPAGE
Lloh1:
	ldr	x8, [x8, ___stderrp@GOTPAGEOFF]
Lloh2:
	ldr	x3, [x8]
Lloh3:
	adrp	x0, l_.str@PAGE
Lloh4:
	add	x0, x0, l_.str@PAGEOFF
	mov	w19, #1                         ; =0x1
	mov	w1, #21                         ; =0x15
	mov	w2, #1                          ; =0x1
	bl	_fwrite
	b	LBB0_7
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	x20, x0
	add	w21, w0, #1
	sxtw	x0, w21
	mov	w22, #1                         ; =0x1
	mov	w1, #1                          ; =0x1
	bl	_calloc
	cbz	x0, LBB0_6
; %bb.3:
	mov	x19, x0
	cmp	w20, #2
	b.ge	LBB0_8
; %bb.4:
	mov	w8, #0                          ; =0x0
LBB0_5:
	str	x8, [sp]
Lloh5:
	adrp	x0, l_.str.1@PAGE
Lloh6:
	add	x0, x0, l_.str.1@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	w19, #0                         ; =0x0
	b	LBB0_7
LBB0_6:
	mov	x19, x22
LBB0_7:
	mov	x0, x19
	ldp	x29, x30, [sp, #80]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #64]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #48]             ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #32]             ; 16-byte Folded Reload
	ldp	x26, x25, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #96
	ret
LBB0_8:
	mov	w8, #0                          ; =0x0
	mov	w9, w20
	add	x10, x9, #1
	add	x11, x19, #10
	add	x12, x19, #8
	add	x13, x19, #4
	add	x14, x19, #6
	mov	x15, #-6                        ; =0xfffffffffffffffa
	mov	w16, #2                         ; =0x2
	mov	w17, #8                         ; =0x8
	mov	w4, #7                          ; =0x7
	mov	w3, #5                          ; =0x5
	mov	w0, #6                          ; =0x6
	mov	w1, #1                          ; =0x1
	mov	x2, #-6                         ; =0xfffffffffffffffa
	mov	w5, #6                          ; =0x6
	mov	w6, #8                          ; =0x8
	b	LBB0_10
LBB0_9:                                 ;   in Loop: Header=BB0_10 Depth=1
	add	x16, x16, #1
	add	x0, x0, x5
	add	x15, x15, x2
	add	x11, x11, x6
	add	x6, x6, #2
	add	x17, x17, #4
	add	x12, x12, x4
	add	x4, x4, #2
	add	x13, x13, x3
	add	x3, x3, #2
	add	x14, x14, x5
	add	x5, x5, #2
	sub	x2, x2, #2
	cmp	x16, x21
	b.eq	LBB0_5
LBB0_10:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_14 Depth 2
                                        ;     Child Loop BB0_16 Depth 2
	cmp	x0, x10
	csel	x24, x0, x10, hi
	adds	x20, x24, x15
	cset	w22, ne
	csetm	x25, ne
	ldrb	w7, [x19, x16]
	cbnz	w7, LBB0_9
; %bb.11:                               ;   in Loop: Header=BB0_10 Depth=1
	add	w8, w8, #1
	mul	x7, x16, x16
	cmp	x7, x9
	b.hi	LBB0_9
; %bb.12:                               ;   in Loop: Header=BB0_10 Depth=1
	sub	x22, x20, x22
	cmp	x20, #0
	cinc	x26, x1, ne
	udiv	x20, x22, x16
	add	x20, x26, x20
	cmp	x20, #4
	b.lo	LBB0_16
; %bb.13:                               ;   in Loop: Header=BB0_10 Depth=1
	mov	x23, #0                         ; =0x0
	and	x22, x20, #0xfffffffffffffffc
	add	x7, x16, x22
	mul	x7, x16, x7
	add	x24, x24, x15
	add	x24, x24, x25
	udiv	x24, x24, x16
	add	x24, x26, x24
	and	x24, x24, #0xfffffffffffffffc
LBB0_14:                                ;   Parent Loop BB0_10 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	strb	w1, [x13, x23]
	strb	w1, [x14, x23]
	strb	w1, [x12, x23]
	strb	w1, [x11, x23]
	add	x23, x23, x17
	subs	x24, x24, #4
	b.ne	LBB0_14
; %bb.15:                               ;   in Loop: Header=BB0_10 Depth=1
	cmp	x20, x22
	b.eq	LBB0_9
LBB0_16:                                ;   Parent Loop BB0_10 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	strb	w1, [x19, x7]
	add	x7, x7, x16
	cmp	x7, x9
	b.ls	LBB0_16
	b	LBB0_9
	.loh AdrpAdd	Lloh3, Lloh4
	.loh AdrpLdrGotLdr	Lloh0, Lloh1, Lloh2
	.loh AdrpAdd	Lloh5, Lloh6
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"Usage: sieve <limit>\n"

l_.str.1:                               ; @.str.1
	.asciz	"%d\n"

.subsections_via_symbols

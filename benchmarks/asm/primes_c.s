	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	sub	sp, sp, #48
	stp	x20, x19, [sp, #16]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #32]             ; 16-byte Folded Spill
	add	x29, sp, #32
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
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
	mov	w1, #18                         ; =0x12
	mov	w2, #1                          ; =0x1
	bl	_fwrite
	b	LBB0_5
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atol
	cmp	x0, #2
	b.ge	LBB0_6
; %bb.3:
	mov	x8, #0                          ; =0x0
LBB0_4:
	str	x8, [sp]
Lloh5:
	adrp	x0, l_.str.1@PAGE
Lloh6:
	add	x0, x0, l_.str.1@PAGEOFF
	bl	_printf
	mov	w19, #0                         ; =0x0
LBB0_5:
	mov	x0, x19
	ldp	x29, x30, [sp, #32]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #48
	ret
LBB0_6:
	mov	x8, #0                          ; =0x0
	mov	w9, #2                          ; =0x2
	b	LBB0_9
LBB0_7:                                 ;   in Loop: Header=BB0_9 Depth=1
	mov	w10, #1                         ; =0x1
LBB0_8:                                 ;   in Loop: Header=BB0_9 Depth=1
	add	x8, x8, x10
	add	x10, x9, #1
	cmp	x9, x0
	mov	x9, x10
	b.eq	LBB0_4
LBB0_9:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_11 Depth 2
	cmp	x9, #4
	b.lo	LBB0_7
; %bb.10:                               ;   in Loop: Header=BB0_9 Depth=1
	mov	w10, #3                         ; =0x3
LBB0_11:                                ;   Parent Loop BB0_9 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	sub	x11, x10, #1
	udiv	x12, x9, x11
	msub	x11, x12, x11, x9
	cbz	x11, LBB0_13
; %bb.12:                               ;   in Loop: Header=BB0_11 Depth=2
	mul	x11, x10, x10
	add	x10, x10, #1
	cmp	x11, x9
	b.ls	LBB0_11
	b	LBB0_7
LBB0_13:                                ;   in Loop: Header=BB0_9 Depth=1
	mov	x10, #0                         ; =0x0
	b	LBB0_8
	.loh AdrpAdd	Lloh3, Lloh4
	.loh AdrpLdrGotLdr	Lloh0, Lloh1, Lloh2
	.loh AdrpAdd	Lloh5, Lloh6
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"Usage: primes <n>\n"

l_.str.1:                               ; @.str.1
	.asciz	"%ld\n"

.subsections_via_symbols

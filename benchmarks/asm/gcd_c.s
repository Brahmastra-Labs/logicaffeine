	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_gcd                            ; -- Begin function gcd
	.p2align	2
_gcd:                                   ; @gcd
	.cfi_startproc
; %bb.0:
	cmp	x1, #1
	b.lt	LBB0_3
LBB0_1:                                 ; =>This Inner Loop Header: Depth=1
	mov	x8, x1
	sdiv	x9, x0, x1
	msub	x1, x9, x1, x0
	mov	x0, x8
	cmp	x1, #0
	b.gt	LBB0_1
; %bb.2:
	mov	x0, x8
LBB0_3:
	ret
	.cfi_endproc
                                        ; -- End function
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
	b.gt	LBB1_2
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
	mov	w1, #15                         ; =0xf
	mov	w2, #1                          ; =0x1
	bl	_fwrite
	b	LBB1_11
LBB1_2:
	ldr	x0, [x1, #8]
	bl	_atol
	cmp	x0, #1
	b.lt	LBB1_9
; %bb.3:
	mov	x8, #0                          ; =0x0
	mov	w9, #1                          ; =0x1
LBB1_4:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB1_5 Depth 2
                                        ;       Child Loop BB1_6 Depth 3
	mov	x10, x9
LBB1_5:                                 ;   Parent Loop BB1_4 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB1_6 Depth 3
	mov	x12, x9
	mov	x13, x10
LBB1_6:                                 ;   Parent Loop BB1_4 Depth=1
                                        ;     Parent Loop BB1_5 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	mov	x11, x13
	sdiv	x13, x12, x13
	msub	x13, x13, x11, x12
	mov	x12, x11
	cmp	x13, #0
	b.gt	LBB1_6
; %bb.7:                                ;   in Loop: Header=BB1_5 Depth=2
	add	x8, x11, x8
	add	x11, x10, #1
	cmp	x10, x0
	mov	x10, x11
	b.lt	LBB1_5
; %bb.8:                                ;   in Loop: Header=BB1_4 Depth=1
	add	x10, x9, #1
	cmp	x9, x0
	mov	x9, x10
	b.ne	LBB1_4
	b	LBB1_10
LBB1_9:
	mov	x8, #0                          ; =0x0
LBB1_10:
	str	x8, [sp]
Lloh5:
	adrp	x0, l_.str.1@PAGE
Lloh6:
	add	x0, x0, l_.str.1@PAGEOFF
	bl	_printf
	mov	w19, #0                         ; =0x0
LBB1_11:
	mov	x0, x19
	ldp	x29, x30, [sp, #32]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #48
	ret
	.loh AdrpAdd	Lloh3, Lloh4
	.loh AdrpLdrGotLdr	Lloh0, Lloh1, Lloh2
	.loh AdrpAdd	Lloh5, Lloh6
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"Usage: gcd <n>\n"

l_.str.1:                               ; @.str.1
	.asciz	"%ld\n"

.subsections_via_symbols

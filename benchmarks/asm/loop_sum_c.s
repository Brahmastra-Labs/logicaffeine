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
	mov	w1, #20                         ; =0x14
	mov	w2, #1                          ; =0x1
	bl	_fwrite
	b	LBB0_7
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atol
	cmp	x0, #1
	b.lt	LBB0_5
; %bb.3:
	mov	x9, #0                          ; =0x0
	mov	x8, #0                          ; =0x0
	mov	x10, #36837                     ; =0x8fe5
	movk	x10, #4770, lsl #16
	movk	x10, #24369, lsl #32
	movk	x10, #35184, lsl #48
	mov	w11, #51719                     ; =0xca07
	movk	w11, #15258, lsl #16
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	add	x9, x9, #1
	add	x8, x9, x8
	smulh	x12, x8, x10
	add	x12, x12, x8
	asr	x13, x12, #29
	add	x12, x13, x12, lsr #63
	msub	x8, x12, x11, x8
	cmp	x0, x9
	b.ne	LBB0_4
	b	LBB0_6
LBB0_5:
	mov	x8, #0                          ; =0x0
LBB0_6:
	str	x8, [sp]
Lloh5:
	adrp	x0, l_.str.1@PAGE
Lloh6:
	add	x0, x0, l_.str.1@PAGEOFF
	bl	_printf
	mov	w19, #0                         ; =0x0
LBB0_7:
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
	.asciz	"Usage: loop_sum <n>\n"

l_.str.1:                               ; @.str.1
	.asciz	"%ld\n"

.subsections_via_symbols

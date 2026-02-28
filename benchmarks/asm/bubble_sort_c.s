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
	mov	w1, #23                         ; =0x17
	mov	w2, #1                          ; =0x1
	bl	_fwrite
	b	LBB0_17
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	x20, x0
	sbfiz	x0, x20, #2, #32
	bl	_malloc
	cbz	x0, LBB0_16
; %bb.3:
	mov	x19, x0
	subs	w8, w20, #1
	b.lt	LBB0_15
; %bb.4:
	mov	x9, #0                          ; =0x0
	mov	x10, x20
	ubfiz	x10, x10, #2, #32
	mov	w11, #42                        ; =0x2a
	mov	w12, #20077                     ; =0x4e6d
	movk	w12, #16838, lsl #16
	mov	w13, #12345                     ; =0x3039
LBB0_5:                                 ; =>This Inner Loop Header: Depth=1
	madd	w11, w11, w12, w13
	ubfx	w14, w11, #16, #15
	str	w14, [x19, x9]
	add	x9, x9, #4
	cmp	x10, x9
	b.ne	LBB0_5
; %bb.6:
	cmp	w20, #2
	b.lt	LBB0_15
; %bb.7:
	mov	w9, #0                          ; =0x0
	add	x10, x19, #4
	mov	x11, x8
	b	LBB0_9
LBB0_8:                                 ;   in Loop: Header=BB0_9 Depth=1
	add	w9, w9, #1
	sub	w11, w11, #1
	cmp	w9, w8
	b.eq	LBB0_15
LBB0_9:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_13 Depth 2
	mov	w11, w11
	cmp	w8, w9
	b.le	LBB0_8
; %bb.10:                               ;   in Loop: Header=BB0_9 Depth=1
	ldr	w12, [x19]
	mov	x13, x11
	mov	x14, x10
	b	LBB0_13
LBB0_11:                                ;   in Loop: Header=BB0_13 Depth=2
	stp	w15, w12, [x14, #-4]
LBB0_12:                                ;   in Loop: Header=BB0_13 Depth=2
	add	x14, x14, #4
	subs	x13, x13, #1
	b.eq	LBB0_8
LBB0_13:                                ;   Parent Loop BB0_9 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldr	w15, [x14]
	cmp	w12, w15
	b.gt	LBB0_11
; %bb.14:                               ;   in Loop: Header=BB0_13 Depth=2
	mov	x12, x15
	b	LBB0_12
LBB0_15:
	ldr	w8, [x19]
	str	x8, [sp]
Lloh5:
	adrp	x0, l_.str.1@PAGE
Lloh6:
	add	x0, x0, l_.str.1@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	w19, #0                         ; =0x0
	b	LBB0_17
LBB0_16:
	mov	w19, #1                         ; =0x1
LBB0_17:
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
	.asciz	"Usage: bubble_sort <n>\n"

l_.str.1:                               ; @.str.1
	.asciz	"%d\n"

.subsections_via_symbols

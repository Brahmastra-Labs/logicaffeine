	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	sub	sp, sp, #64
	stp	x22, x21, [sp, #16]             ; 16-byte Folded Spill
	stp	x20, x19, [sp, #32]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #48]             ; 16-byte Folded Spill
	add	x29, sp, #48
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
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
	mov	w1, #19                         ; =0x13
	mov	w2, #1                          ; =0x1
	bl	_fwrite
	b	LBB0_20
LBB0_2:
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	x19, x0
	lsl	w8, w0, #1
	sub	w8, w8, #1
	orr	w8, w8, w8, lsr #1
	orr	w8, w8, w8, lsr #2
	orr	w8, w8, w8, lsr #4
	orr	w8, w8, w8, lsr #8
	orr	w8, w8, w8, lsr #16
	add	w9, w8, #1
	cmp	w9, #16
	mov	w9, #16                         ; =0x10
	csinc	w20, w9, w8, ls
	mov	w22, #12                        ; =0xc
	mov	x0, x20
	mov	w1, #12                         ; =0xc
	bl	_calloc
	adrp	x21, _table@PAGE
	str	x0, [x21, _table@PAGEOFF]
	cmp	w19, #1
	b.lt	LBB0_18
; %bb.3:
	mov	w10, #0                         ; =0x0
	sub	w8, w20, #1
	mov	w9, #40763                      ; =0x9f3b
	movk	w9, #1117, lsl #16
	mov	w11, #1                         ; =0x1
	b	LBB0_5
LBB0_4:                                 ;   in Loop: Header=BB0_5 Depth=1
	lsl	w14, w10, #1
	umaddl	x12, w12, w22, x0
	stp	w10, w14, [x12]
	str	w11, [x13]
	add	w10, w10, #1
	cmp	w10, w19
	b.eq	LBB0_9
LBB0_5:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_6 Depth 2
	eor	w12, w10, w10, lsr #16
	mul	w12, w12, w9
	eor	w12, w12, w12, lsr #16
LBB0_6:                                 ;   Parent Loop BB0_5 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	and	w12, w12, w8
	umaddl	x13, w12, w22, x0
	ldr	w14, [x13, #8]!
	cbz	w14, LBB0_4
; %bb.7:                                ;   in Loop: Header=BB0_6 Depth=2
	umull	x14, w12, w22
	ldr	w14, [x0, x14]
	cmp	w14, w10
	b.eq	LBB0_4
; %bb.8:                                ;   in Loop: Header=BB0_6 Depth=2
	add	w12, w12, #1
	b	LBB0_6
LBB0_9:
	cmp	w19, #1
	b.lt	LBB0_18
; %bb.10:
	mov	w11, #0                         ; =0x0
	mov	w10, #0                         ; =0x0
	mov	w12, #12                        ; =0xc
	b	LBB0_13
LBB0_11:                                ;   in Loop: Header=BB0_13 Depth=1
	mov	w13, #-1                        ; =0xffffffff
LBB0_12:                                ;   in Loop: Header=BB0_13 Depth=1
	cmp	w13, w11, lsl #1
	cinc	w10, w10, eq
	add	w11, w11, #1
	cmp	w11, w19
	b.eq	LBB0_19
LBB0_13:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_14 Depth 2
	eor	w13, w11, w11, lsr #16
	mul	w13, w13, w9
	eor	w13, w13, w13, lsr #16
LBB0_14:                                ;   Parent Loop BB0_13 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	and	w13, w13, w8
	umaddl	x14, w13, w12, x0
	ldr	w14, [x14, #8]
	cbz	w14, LBB0_11
; %bb.15:                               ;   in Loop: Header=BB0_14 Depth=2
	mul	x14, x13, x12
	ldr	w14, [x0, x14]
	cmp	w14, w11
	b.eq	LBB0_17
; %bb.16:                               ;   in Loop: Header=BB0_14 Depth=2
	add	w13, w13, #1
	b	LBB0_14
LBB0_17:                                ;   in Loop: Header=BB0_13 Depth=1
	madd	x13, x13, x12, x0
	ldr	w13, [x13, #4]
	b	LBB0_12
LBB0_18:
	mov	w10, #0                         ; =0x0
LBB0_19:
	str	x10, [sp]
Lloh5:
	adrp	x0, l_.str.1@PAGE
Lloh6:
	add	x0, x0, l_.str.1@PAGEOFF
	bl	_printf
	ldr	x0, [x21, _table@PAGEOFF]
	bl	_free
	mov	w19, #0                         ; =0x0
LBB0_20:
	mov	x0, x19
	ldp	x29, x30, [sp, #48]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #32]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #64
	ret
	.loh AdrpAdd	Lloh3, Lloh4
	.loh AdrpLdrGotLdr	Lloh0, Lloh1, Lloh2
	.loh AdrpAdd	Lloh5, Lloh6
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"Usage: collect <n>\n"

.zerofill __DATA,__bss,_table,8,3       ; @table
l_.str.1:                               ; @.str.1
	.asciz	"%d\n"

.subsections_via_symbols

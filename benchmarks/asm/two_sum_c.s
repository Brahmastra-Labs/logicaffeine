	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_ht_contains                    ; -- Begin function ht_contains
	.p2align	2
_ht_contains:                           ; @ht_contains
	.cfi_startproc
; %bb.0:
	mov	w8, #31153                      ; =0x79b1
	movk	w8, #40503, lsl #16
	mul	x8, x2, x8
	and	x8, x8, x1
	add	x9, x0, x8, lsl #4
	ldr	w9, [x9, #8]
	cbz	w9, LBB0_3
LBB0_1:                                 ; =>This Inner Loop Header: Depth=1
	lsl	x9, x8, #4
	ldr	x9, [x0, x9]
	cmp	x9, x2
	b.eq	LBB0_4
; %bb.2:                                ;   in Loop: Header=BB0_1 Depth=1
	add	x8, x8, #1
	and	x8, x8, x1
	add	x9, x0, x8, lsl #4
	ldr	w9, [x9, #8]
	cbnz	w9, LBB0_1
LBB0_3:
	mov	w0, #0                          ; =0x0
	ret
LBB0_4:
	mov	w0, #1                          ; =0x1
	ret
	.cfi_endproc
                                        ; -- End function
	.globl	_ht_insert                      ; -- Begin function ht_insert
	.p2align	2
_ht_insert:                             ; @ht_insert
	.cfi_startproc
; %bb.0:
	mov	w8, #31153                      ; =0x79b1
	movk	w8, #40503, lsl #16
	mul	x8, x2, x8
LBB1_1:                                 ; =>This Inner Loop Header: Depth=1
	and	x9, x8, x1
	add	x8, x0, x9, lsl #4
	mov	x10, x8
	ldr	w11, [x10, #8]!
	cbz	w11, LBB1_4
; %bb.2:                                ;   in Loop: Header=BB1_1 Depth=1
	ldr	x8, [x8]
	cmp	x8, x2
	b.eq	LBB1_5
; %bb.3:                                ;   in Loop: Header=BB1_1 Depth=1
	add	x8, x9, #1
	b	LBB1_1
LBB1_4:
	str	x2, [x8]
	mov	w8, #1                          ; =0x1
	str	w8, [x10]
LBB1_5:
	ret
	.cfi_endproc
                                        ; -- End function
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	cmp	w0, #2
	b.ge	LBB2_2
; %bb.1:
	mov	w0, #1                          ; =0x1
	ret
LBB2_2:
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
	ldr	x0, [x1, #8]
	bl	_atol
	mov	x20, x0
	lsl	x0, x0, #3
	bl	_malloc
	mov	x19, x0
	cmp	x20, #1
	b.lt	LBB2_5
; %bb.3:
	mov	x8, #0                          ; =0x0
	mov	w9, #42                         ; =0x2a
	mov	w10, #20077                     ; =0x4e6d
	movk	w10, #16838, lsl #16
	mov	w11, #12345                     ; =0x3039
LBB2_4:                                 ; =>This Inner Loop Header: Depth=1
	madd	x12, x9, x10, x11
	and	x9, x12, #0x7fffffff
	ubfx	x12, x12, #16, #15
	sdiv	x13, x12, x20
	msub	x12, x13, x20, x12
	str	x12, [x19, x8, lsl #3]
	add	x8, x8, #1
	cmp	x20, x8
	b.ne	LBB2_4
LBB2_5:
	lsl	x8, x20, #1
	sub	x8, x8, #1
	orr	x8, x8, x8, lsr #1
	orr	x8, x8, x8, lsr #2
	orr	x8, x8, x8, lsr #4
	orr	x8, x8, x8, lsr #8
	orr	x8, x8, x8, lsr #16
	orr	x8, x8, x8, lsr #32
	add	x9, x8, #1
	cmp	x9, #16
	mov	w9, #16                         ; =0x10
	csinc	x22, x9, x8, ls
	mov	x0, x22
	mov	w1, #16                         ; =0x10
	bl	_calloc
	mov	x21, x0
	cmp	x20, #1
	b.lt	LBB2_19
; %bb.6:
	mov	x9, #0                          ; =0x0
	mov	x8, #0                          ; =0x0
	sub	x10, x22, #1
	mov	w11, #31153                     ; =0x79b1
	movk	w11, #40503, lsl #16
	mov	w12, #1                         ; =0x1
	b	LBB2_9
LBB2_7:                                 ;   in Loop: Header=BB2_9 Depth=1
	str	x13, [x14]
	str	w12, [x16]
LBB2_8:                                 ;   in Loop: Header=BB2_9 Depth=1
	add	x9, x9, #1
	cmp	x9, x20
	b.eq	LBB2_20
LBB2_9:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB2_11 Depth 2
                                        ;     Child Loop BB2_16 Depth 2
	ldr	x13, [x19, x9, lsl #3]
	subs	x14, x20, x13
	b.mi	LBB2_15
; %bb.10:                               ;   in Loop: Header=BB2_9 Depth=1
	mul	x15, x14, x11
LBB2_11:                                ;   Parent Loop BB2_9 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	and	x15, x15, x10
	add	x16, x21, x15, lsl #4
	ldr	w16, [x16, #8]
	cbz	w16, LBB2_15
; %bb.12:                               ;   in Loop: Header=BB2_11 Depth=2
	lsl	x16, x15, #4
	ldr	x16, [x21, x16]
	cmp	x16, x14
	b.eq	LBB2_14
; %bb.13:                               ;   in Loop: Header=BB2_11 Depth=2
	add	x15, x15, #1
	b	LBB2_11
LBB2_14:                                ;   in Loop: Header=BB2_9 Depth=1
	add	x8, x8, #1
LBB2_15:                                ;   in Loop: Header=BB2_9 Depth=1
	mul	x14, x13, x11
LBB2_16:                                ;   Parent Loop BB2_9 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	and	x15, x14, x10
	add	x14, x21, x15, lsl #4
	mov	x16, x14
	ldr	w17, [x16, #8]!
	cbz	w17, LBB2_7
; %bb.17:                               ;   in Loop: Header=BB2_16 Depth=2
	ldr	x14, [x14]
	cmp	x14, x13
	b.eq	LBB2_8
; %bb.18:                               ;   in Loop: Header=BB2_16 Depth=2
	add	x14, x15, #1
	b	LBB2_16
LBB2_19:
	mov	x8, #0                          ; =0x0
LBB2_20:
	str	x8, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	x0, x21
	bl	_free
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #48]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #32]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #64
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%ld\n"

.subsections_via_symbols

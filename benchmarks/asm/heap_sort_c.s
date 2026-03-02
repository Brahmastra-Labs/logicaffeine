	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_sift_down                      ; -- Begin function sift_down
	.p2align	2
_sift_down:                             ; @sift_down
	.cfi_startproc
; %bb.0:
	mov	w10, #1                         ; =0x1
	bfi	x10, x1, #1, #63
	cmp	x10, x2
	b.gt	LBB0_6
; %bb.1:
	lsl	x11, x1, #1
	ldr	x8, [x0, x1, lsl #3]
LBB0_2:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x9, [x0, x10, lsl #3]
	cmp	x8, x9
	csel	x9, x10, x1, lt
	add	x10, x11, #2
	cmp	x10, x2
	b.gt	LBB0_4
; %bb.3:                                ;   in Loop: Header=BB0_2 Depth=1
	ldr	x11, [x0, x9, lsl #3]
	ldr	x12, [x0, x10, lsl #3]
	cmp	x11, x12
	csel	x9, x10, x9, lt
LBB0_4:                                 ;   in Loop: Header=BB0_2 Depth=1
	cmp	x9, x1
	b.eq	LBB0_6
; %bb.5:                                ;   in Loop: Header=BB0_2 Depth=1
	lsl	x10, x9, #3
	ldr	x11, [x0, x10]
	str	x11, [x0, x1, lsl #3]
	str	x8, [x0, x10]
	lsl	x11, x9, #1
	mov	w10, #1                         ; =0x1
	bfi	x10, x9, #1, #63
	mov	x1, x9
	cmp	x10, x2
	b.le	LBB0_2
LBB0_6:
	ret
	.cfi_endproc
                                        ; -- End function
	.globl	_heap_sort                      ; -- Begin function heap_sort
	.p2align	2
_heap_sort:                             ; @heap_sort
	.cfi_startproc
; %bb.0:
	subs	x8, x1, #1
	b.lt	LBB1_10
; %bb.1:
	subs	x9, x1, #2
	csel	x8, x8, x9, lt
	asr	x8, x8, #1
	b	LBB1_3
LBB1_2:                                 ;   in Loop: Header=BB1_3 Depth=1
	sub	x9, x8, #1
	cmp	x8, #0
	mov	x8, x9
	b.le	LBB1_9
LBB1_3:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB1_5 Depth 2
	mov	w12, #1                         ; =0x1
	bfi	x12, x8, #1, #63
	cmp	x12, x1
	b.ge	LBB1_2
; %bb.4:                                ;   in Loop: Header=BB1_3 Depth=1
	lsl	x13, x8, #1
	ldr	x9, [x0, x8, lsl #3]
	mov	x11, x8
LBB1_5:                                 ;   Parent Loop BB1_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldr	x10, [x0, x12, lsl #3]
	cmp	x9, x10
	csel	x10, x12, x11, lt
	add	x12, x13, #2
	cmp	x12, x1
	b.ge	LBB1_7
; %bb.6:                                ;   in Loop: Header=BB1_5 Depth=2
	ldr	x13, [x0, x10, lsl #3]
	ldr	x14, [x0, x12, lsl #3]
	cmp	x13, x14
	csel	x10, x12, x10, lt
LBB1_7:                                 ;   in Loop: Header=BB1_5 Depth=2
	cmp	x10, x11
	b.eq	LBB1_2
; %bb.8:                                ;   in Loop: Header=BB1_5 Depth=2
	lsl	x12, x10, #3
	ldr	x13, [x0, x12]
	str	x13, [x0, x11, lsl #3]
	str	x9, [x0, x12]
	lsl	x13, x10, #1
	mov	w12, #1                         ; =0x1
	bfi	x12, x10, #1, #63
	mov	x11, x10
	cmp	x12, x1
	b.lt	LBB1_5
	b	LBB1_2
LBB1_9:
	cmp	x1, #2
	b.ge	LBB1_12
LBB1_10:
	ret
LBB1_11:                                ;   in Loop: Header=BB1_12 Depth=1
	cmp	x1, #2
	mov	x1, x8
	b.le	LBB1_10
LBB1_12:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB1_14 Depth 2
	sub	x8, x1, #1
	ldr	x9, [x0]
	lsl	x10, x8, #3
	ldr	x11, [x0, x10]
	str	x11, [x0]
	str	x9, [x0, x10]
	subs	x9, x1, #2
	b.eq	LBB1_10
; %bb.13:                               ;   in Loop: Header=BB1_12 Depth=1
	mov	x13, #0                         ; =0x0
	mov	x11, #0                         ; =0x0
	ldr	x10, [x0]
	mov	w14, #1                         ; =0x1
LBB1_14:                                ;   Parent Loop BB1_12 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldr	x12, [x0, x14, lsl #3]
	cmp	x10, x12
	csel	x12, x14, x11, lt
	add	x13, x13, #2
	cmp	x13, x9
	b.gt	LBB1_16
; %bb.15:                               ;   in Loop: Header=BB1_14 Depth=2
	ldr	x14, [x0, x12, lsl #3]
	ldr	x15, [x0, x13, lsl #3]
	cmp	x14, x15
	csel	x12, x13, x12, lt
LBB1_16:                                ;   in Loop: Header=BB1_14 Depth=2
	cmp	x12, x11
	b.eq	LBB1_11
; %bb.17:                               ;   in Loop: Header=BB1_14 Depth=2
	lsl	x13, x12, #3
	ldr	x14, [x0, x13]
	str	x14, [x0, x11, lsl #3]
	str	x10, [x0, x13]
	lsl	x13, x12, #1
	mov	w14, #1                         ; =0x1
	bfi	x14, x12, #1, #63
	mov	x11, x12
	cmp	x14, x9
	b.le	LBB1_14
	b	LBB1_11
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
	stp	x20, x19, [sp, #32]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #48]             ; 16-byte Folded Spill
	add	x29, sp, #48
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
	cmp	x20, #1
	b.lt	LBB2_8
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
	str	x12, [x19, x8, lsl #3]
	add	x8, x8, #1
	cmp	x20, x8
	b.ne	LBB2_4
; %bb.5:
	mov	x0, x19
	mov	x1, x20
	bl	_heap_sort
	cmp	x20, #1
	b.lt	LBB2_9
; %bb.6:
	mov	x8, #0                          ; =0x0
	mov	x9, #36837                      ; =0x8fe5
	movk	x9, #4770, lsl #16
	movk	x9, #24369, lsl #32
	movk	x9, #35184, lsl #48
	mov	w10, #51719                     ; =0xca07
	movk	w10, #15258, lsl #16
	mov	x11, x19
	mov	x12, x20
LBB2_7:                                 ; =>This Inner Loop Header: Depth=1
	ldr	x13, [x11], #8
	add	x8, x13, x8
	smulh	x13, x8, x9
	add	x13, x13, x8
	asr	x14, x13, #29
	add	x13, x14, x13, lsr #63
	msub	x8, x13, x10, x8
	subs	x12, x12, #1
	b.ne	LBB2_7
	b	LBB2_10
LBB2_8:
	mov	x0, x19
	mov	x1, x20
	bl	_heap_sort
LBB2_9:
	mov	x8, #0                          ; =0x0
LBB2_10:
	ldr	x9, [x19]
	add	x10, x19, x20, lsl #3
	ldur	x10, [x10, #-8]
	stp	x10, x8, [sp, #8]
	str	x9, [sp]
Lloh0:
	adrp	x0, l_.str@PAGE
Lloh1:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #48]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #32]             ; 16-byte Folded Reload
	add	sp, sp, #64
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%ld %ld %ld\n"

.subsections_via_symbols

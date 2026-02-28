	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.section	__TEXT,__literal16,16byte_literals
	.p2align	4, 0x0                          ; -- Begin function main
lCPI0_0:
	.long	0                               ; 0x0
	.long	1                               ; 0x1
	.long	2                               ; 0x2
	.long	3                               ; 0x3
lCPI0_1:
	.long	0                               ; 0x0
	.long	4294967295                      ; 0xffffffff
	.long	4294967294                      ; 0xfffffffe
	.long	4294967293                      ; 0xfffffffd
	.section	__TEXT,__text,regular,pure_instructions
	.globl	_main
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
	sub	sp, sp, #160
	stp	x28, x27, [sp, #64]             ; 16-byte Folded Spill
	stp	x26, x25, [sp, #80]             ; 16-byte Folded Spill
	stp	x24, x23, [sp, #96]             ; 16-byte Folded Spill
	stp	x22, x21, [sp, #112]            ; 16-byte Folded Spill
	stp	x20, x19, [sp, #128]            ; 16-byte Folded Spill
	stp	x29, x30, [sp, #144]            ; 16-byte Folded Spill
	add	x29, sp, #144
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
	.cfi_offset w27, -88
	.cfi_offset w28, -96
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	x23, x0
	sbfiz	x22, x23, #2, #32
	mov	x0, x22
	bl	_malloc
	mov	x19, x0
	mov	x0, x22
	bl	_malloc
	mov	x20, x0
	mov	x0, x22
	bl	_malloc
	mov	x21, x0
	mov	w28, w23
	cmp	w23, #1
	b.lt	LBB0_9
; %bb.3:
	cmp	w23, #16
	b.hs	LBB0_5
; %bb.4:
	mov	x8, #0                          ; =0x0
	b	LBB0_8
LBB0_5:
	and	x8, x28, #0xfffffff0
Lloh0:
	adrp	x9, lCPI0_0@PAGE
Lloh1:
	ldr	q0, [x9, lCPI0_0@PAGEOFF]
	add	x9, x20, #32
	movi.4s	v1, #4
	movi.4s	v2, #8
	movi.4s	v3, #12
	movi.4s	v4, #16
	mov	x10, x8
LBB0_6:                                 ; =>This Inner Loop Header: Depth=1
	add.4s	v5, v0, v1
	add.4s	v6, v0, v2
	add.4s	v7, v0, v3
	stp	q0, q5, [x9, #-32]
	stp	q6, q7, [x9], #64
	add.4s	v0, v0, v4
	subs	x10, x10, #16
	b.ne	LBB0_6
; %bb.7:
	cmp	x8, x28
	b.eq	LBB0_9
LBB0_8:                                 ; =>This Inner Loop Header: Depth=1
	str	w8, [x20, x8, lsl #2]
	add	x8, x8, #1
	cmp	x28, x8
	b.ne	LBB0_8
LBB0_9:
	mov	w25, #0                         ; =0x0
	str	wzr, [sp, #60]                  ; 4-byte Folded Spill
	mov	w26, #0                         ; =0x0
Lloh2:
	adrp	x8, lCPI0_1@PAGE
Lloh3:
	ldr	q0, [x8, lCPI0_1@PAGEOFF]
	str	q0, [sp, #16]                   ; 16-byte Folded Spill
	mvni.4s	v5, #3
	add	x24, x20, #4
	mvni.4s	v6, #7
	mvni.4s	v7, #11
	mvni.4s	v16, #15
	str	x22, [sp, #32]                  ; 8-byte Folded Spill
	cmp	w23, #2
	b.lt	LBB0_17
LBB0_10:
	mov	w8, w23
	cmp	w23, #25
	b.lo	LBB0_16
; %bb.11:
	sub	x9, x8, #2
	sub	w10, w23, #1
	cmp	w10, w9
	b.lo	LBB0_16
; %bb.12:
	lsr	x9, x9, #32
	cbnz	x9, LBB0_16
; %bb.13:
	sub	x9, x8, #1
	and	x10, x9, #0xfffffffffffffff0
	sub	x8, x8, x10
	dup.4s	v0, w23
	ldr	q1, [sp, #16]                   ; 16-byte Folded Reload
	add.4s	v0, v0, v1
	mov	x11, x10
	mov	x12, x9
LBB0_14:                                ; =>This Inner Loop Header: Depth=1
	add.4s	v1, v0, v5
	add.4s	v2, v0, v6
	add.4s	v3, v0, v7
	add	x13, x21, w12, uxtw #2
	rev64.4s	v4, v0
	ext.16b	v4, v4, v4, #8
	stur	q4, [x13, #-12]
	rev64.4s	v1, v1
	ext.16b	v1, v1, v1, #8
	stur	q1, [x13, #-28]
	rev64.4s	v1, v2
	ext.16b	v1, v1, v1, #8
	stur	q1, [x13, #-44]
	rev64.4s	v1, v3
	ext.16b	v1, v1, v1, #8
	stur	q1, [x13, #-60]
	add.4s	v0, v0, v16
	sub	x12, x12, #16
	subs	x11, x11, #16
	b.ne	LBB0_14
; %bb.15:
	cmp	x9, x10
	b.eq	LBB0_17
LBB0_16:                                ; =>This Inner Loop Header: Depth=1
	sub	x9, x8, #1
	str	w8, [x21, w9, uxtw #2]
	cmp	x8, #2
	mov	x8, x9
	b.hi	LBB0_16
LBB0_17:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB0_20 Depth 2
                                        ;       Child Loop BB0_22 Depth 3
                                        ;     Child Loop BB0_27 Depth 2
	mov	x0, x19
	mov	x1, x20
	mov	x2, x22
	bl	_memcpy
	ldr	w9, [x19]
	cbz	w9, LBB0_24
; %bb.18:                               ;   in Loop: Header=BB0_17 Depth=1
	mov	w8, #0                          ; =0x0
	mvni.4s	v5, #3
	mvni.4s	v6, #7
	mvni.4s	v7, #11
	mvni.4s	v16, #15
	b	LBB0_20
LBB0_19:                                ;   in Loop: Header=BB0_20 Depth=2
	add	w8, w8, #1
	cbz	x9, LBB0_25
LBB0_20:                                ;   Parent Loop BB0_17 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB0_22 Depth 3
	cmp	w9, #1
	b.lt	LBB0_19
; %bb.21:                               ;   in Loop: Header=BB0_20 Depth=2
	mov	x10, #0                         ; =0x0
	lsl	x11, x9, #1
	lsl	x9, x9, #2
	add	x11, x11, #2
	and	x11, x11, #0xfffffffffffffffc
LBB0_22:                                ;   Parent Loop BB0_17 Depth=1
                                        ;     Parent Loop BB0_20 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	ldr	w12, [x19, x10]
	ldr	w13, [x19, x9]
	str	w13, [x19, x10]
	str	w12, [x19, x9]
	sub	x9, x9, #4
	add	x10, x10, #4
	cmp	x11, x10
	b.ne	LBB0_22
; %bb.23:                               ;   in Loop: Header=BB0_20 Depth=2
	ldr	w9, [x19]
	b	LBB0_19
LBB0_24:                                ;   in Loop: Header=BB0_17 Depth=1
	mov	w8, #0                          ; =0x0
	mvni.4s	v5, #3
	mvni.4s	v6, #7
	mvni.4s	v7, #11
	mvni.4s	v16, #15
LBB0_25:                                ;   in Loop: Header=BB0_17 Depth=1
	cmp	w8, w26
	csel	w26, w8, w26, gt
	ldr	w9, [sp, #60]                   ; 4-byte Folded Reload
	tst	w9, #0x1
	cneg	w8, w8, ne
	add	w25, w8, w25
	stp	x26, x25, [sp, #40]             ; 16-byte Folded Spill
	add	w9, w9, #1
	str	w9, [sp, #60]                   ; 4-byte Folded Spill
	cmp	w23, #1
	csinc	w8, w23, wzr, lt
	sxtw	x23, w8
	sbfiz	x26, x8, #2, #32
	mov	x22, #-1                        ; =0xffffffffffffffff
	mov	x25, x28
	b	LBB0_27
LBB0_26:                                ;   in Loop: Header=BB0_27 Depth=2
	str	w27, [x20, x26]
	ldr	w8, [x21, x26]
	sub	x25, x25, #1
	add	x22, x22, #1
	subs	w8, w8, #1
	str	w8, [x21, x26]
	add	x26, x26, #4
	b.gt	LBB0_30
LBB0_27:                                ;   Parent Loop BB0_17 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	cmp	w23, w25
	b.eq	LBB0_31
; %bb.28:                               ;   in Loop: Header=BB0_27 Depth=2
	add	x8, x23, x22
	add	x8, x8, #1
	ldr	w27, [x20]
	cmp	x8, #1
	b.lt	LBB0_26
; %bb.29:                               ;   in Loop: Header=BB0_27 Depth=2
	and	x2, x26, #0x3fffffffc
	mov	x0, x20
	mov	x1, x24
	bl	_memmove
	mvni.4s	v16, #15
	mvni.4s	v7, #11
	mvni.4s	v6, #7
	mvni.4s	v5, #3
	b	LBB0_26
LBB0_30:                                ;   in Loop: Header=BB0_17 Depth=1
	add	w23, w23, w22
	ldp	x22, x26, [sp, #32]             ; 16-byte Folded Reload
	ldr	x25, [sp, #48]                  ; 8-byte Folded Reload
	cmp	w23, #2
	b.ge	LBB0_10
	b	LBB0_17
LBB0_31:
	ldr	x9, [sp, #40]                   ; 8-byte Folded Reload
	ldr	x8, [sp, #48]                   ; 8-byte Folded Reload
	stp	x8, x9, [sp]
Lloh4:
	adrp	x0, l_.str@PAGE
Lloh5:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	x0, x20
	bl	_free
	mov	x0, x21
	bl	_free
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #144]            ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #128]            ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #112]            ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #96]             ; 16-byte Folded Reload
	ldp	x26, x25, [sp, #80]             ; 16-byte Folded Reload
	ldp	x28, x27, [sp, #64]             ; 16-byte Folded Reload
	add	sp, sp, #160
	ret
	.loh AdrpLdr	Lloh0, Lloh1
	.loh AdrpLdr	Lloh2, Lloh3
	.loh AdrpAdd	Lloh4, Lloh5
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%d\n%d\n"

.subsections_via_symbols

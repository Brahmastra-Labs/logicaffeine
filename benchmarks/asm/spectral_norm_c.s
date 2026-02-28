	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_A                              ; -- Begin function A
	.p2align	2
_A:                                     ; @A
	.cfi_startproc
; %bb.0:
	add	w8, w1, w0
	madd	w8, w8, w8, w8
	cmp	w8, #0
	cinc	w8, w8, lt
	add	w8, w0, w8, asr #1
	add	w8, w8, #1
	scvtf	d0, w8
	fmov	d1, #1.00000000
	fdiv	d0, d1, d0
	ret
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__literal8,8byte_literals
	.p2align	3, 0x0                          ; -- Begin function mul_Av
lCPI1_0:
	.long	0                               ; 0x0
	.long	1                               ; 0x1
	.section	__TEXT,__literal16,16byte_literals
	.p2align	4, 0x0
lCPI1_1:
	.quad	0                               ; 0x0
	.quad	1                               ; 0x1
	.section	__TEXT,__text,regular,pure_instructions
	.globl	_mul_Av
	.p2align	2
_mul_Av:                                ; @mul_Av
	.cfi_startproc
; %bb.0:
	cmp	w0, #1
	b.lt	LBB1_12
; %bb.1:
	stp	d11, d10, [sp, #-32]!           ; 16-byte Folded Spill
	stp	d9, d8, [sp, #16]               ; 16-byte Folded Spill
	.cfi_def_cfa_offset 32
	.cfi_offset b8, -8
	.cfi_offset b9, -16
	.cfi_offset b10, -24
	.cfi_offset b11, -32
	mov	x10, #0                         ; =0x0
	mov	w8, w0
	add	x9, x1, w0, uxtw #3
	and	x11, x8, #0xfffffff8
Lloh0:
	adrp	x12, lCPI1_0@PAGE
Lloh1:
	ldr	d0, [x12, lCPI1_0@PAGEOFF]
	mov	w12, #8                         ; =0x8
	dup.2d	v1, x12
Lloh2:
	adrp	x12, lCPI1_1@PAGE
Lloh3:
	ldr	q2, [x12, lCPI1_1@PAGEOFF]
	add	x12, x1, #32
	fmov	d3, #1.00000000
	movi.2s	v4, #1
	fmov.2d	v5, #1.00000000
	movi.2s	v6, #8
	b	LBB1_3
LBB1_2:                                 ;   in Loop: Header=BB1_3 Depth=1
	mov	x10, x13
	cmp	x13, x8
	b.eq	LBB1_11
LBB1_3:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB1_9 Depth 2
                                        ;     Child Loop BB1_7 Depth 2
	str	xzr, [x2, x10, lsl #3]
	add	x13, x10, #1
	movi	d7, #0000000000000000
	cmp	w0, #8
	b.lo	LBB1_6
; %bb.4:                                ;   in Loop: Header=BB1_3 Depth=1
	add	x14, x2, x10, lsl #3
	cmp	x14, x9
	b.hs	LBB1_8
; %bb.5:                                ;   in Loop: Header=BB1_3 Depth=1
	add	x14, x14, #8
	cmp	x14, x1
	b.ls	LBB1_8
LBB1_6:                                 ;   in Loop: Header=BB1_3 Depth=1
	mov	x14, #0                         ; =0x0
LBB1_7:                                 ;   Parent Loop BB1_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	add	w15, w10, w14
	madd	w15, w15, w15, w15
	add	w15, w13, w15, lsr #1
	scvtf	d16, w15
	ldr	d17, [x1, x14, lsl #3]
	fdiv	d16, d3, d16
	fmadd	d7, d16, d17, d7
	str	d7, [x2, x10, lsl #3]
	add	x14, x14, #1
	cmp	x8, x14
	b.ne	LBB1_7
	b	LBB1_2
LBB1_8:                                 ;   in Loop: Header=BB1_3 Depth=1
	dup.2d	v16, x10
	dup.2s	v17, w10
	add	x14, x10, #2
	dup.2d	v18, x14
	add	x14, x10, #4
	dup.2d	v19, x14
	add	x14, x10, #6
	add	w15, w10, #2
	dup.2d	v20, x14
	dup.2s	v21, w15
	add	w14, w10, #4
	add	w15, w10, #6
	dup.2s	v22, w13
	dup.2s	v23, w14
	mov	x14, x11
	dup.2s	v24, w15
	mov	x15, x12
	mov.16b	v25, v2
	fmov	d26, d0
LBB1_9:                                 ;   Parent Loop BB1_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	add.2d	v27, v25, v16
	add.2d	v28, v18, v25
	add.2d	v29, v19, v25
	add.2d	v30, v20, v25
	add.2s	v31, v26, v17
	add.2s	v8, v21, v26
	add.2s	v9, v23, v26
	add.2s	v10, v24, v26
	add.2s	v31, v31, v4
	add.2s	v8, v8, v4
	add.2s	v9, v9, v4
	add.2s	v10, v10, v4
	xtn.2s	v27, v27
	xtn.2s	v28, v28
	xtn.2s	v29, v29
	xtn.2s	v30, v30
	mul.2s	v27, v31, v27
	mul.2s	v28, v8, v28
	mul.2s	v29, v9, v29
	mul.2s	v30, v10, v30
	fmov	d31, d22
	usra.2s	v31, v27, #1
	fmov	d27, d22
	usra.2s	v27, v28, #1
	fmov	d28, d22
	usra.2s	v28, v29, #1
	fmov	d29, d22
	usra.2s	v29, v30, #1
	sshll.2d	v30, v31, #0
	scvtf.2d	v30, v30
	sshll.2d	v27, v27, #0
	scvtf.2d	v27, v27
	sshll.2d	v28, v28, #0
	scvtf.2d	v28, v28
	sshll.2d	v29, v29, #0
	scvtf.2d	v29, v29
	fdiv.2d	v30, v5, v30
	fdiv.2d	v27, v5, v27
	fdiv.2d	v28, v5, v28
	fdiv.2d	v29, v5, v29
	ldp	q31, q8, [x15, #-32]
	ldp	q9, q10, [x15], #64
	fmul.2d	v30, v30, v31
	mov	d31, v30[1]
	fmul.2d	v27, v27, v8
	mov	d8, v27[1]
	fmul.2d	v28, v28, v9
	mov	d9, v28[1]
	fmul.2d	v29, v29, v10
	mov	d10, v29[1]
	fadd	d7, d7, d30
	fadd	d7, d7, d31
	fadd	d7, d7, d27
	fadd	d7, d7, d8
	fadd	d7, d7, d28
	fadd	d7, d7, d9
	fadd	d7, d7, d29
	fadd	d7, d7, d10
	add.2d	v25, v25, v1
	add.2s	v26, v26, v6
	subs	x14, x14, #8
	b.ne	LBB1_9
; %bb.10:                               ;   in Loop: Header=BB1_3 Depth=1
	str	d7, [x2, x10, lsl #3]
	mov	x14, x11
	cmp	x11, x8
	b.ne	LBB1_7
	b	LBB1_2
LBB1_11:
	ldp	d9, d8, [sp, #16]               ; 16-byte Folded Reload
	ldp	d11, d10, [sp], #32             ; 16-byte Folded Reload
LBB1_12:
	ret
	.loh AdrpLdr	Lloh2, Lloh3
	.loh AdrpLdr	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__literal8,8byte_literals
	.p2align	3, 0x0                          ; -- Begin function mul_Atv
lCPI2_0:
	.long	0                               ; 0x0
	.long	1                               ; 0x1
	.section	__TEXT,__literal16,16byte_literals
	.p2align	4, 0x0
lCPI2_1:
	.quad	0                               ; 0x0
	.quad	1                               ; 0x1
	.section	__TEXT,__text,regular,pure_instructions
	.globl	_mul_Atv
	.p2align	2
_mul_Atv:                               ; @mul_Atv
	.cfi_startproc
; %bb.0:
	cmp	w0, #1
	b.lt	LBB2_13
; %bb.1:
	stp	d13, d12, [sp, #-48]!           ; 16-byte Folded Spill
	stp	d11, d10, [sp, #16]             ; 16-byte Folded Spill
	stp	d9, d8, [sp, #32]               ; 16-byte Folded Spill
	.cfi_def_cfa_offset 48
	.cfi_offset b8, -8
	.cfi_offset b9, -16
	.cfi_offset b10, -24
	.cfi_offset b11, -32
	.cfi_offset b12, -40
	.cfi_offset b13, -48
	mov	x8, #0                          ; =0x0
	mov	w9, w0
	add	x10, x1, w0, uxtw #3
	and	x11, x9, #0xfffffff8
	add	x12, x1, #32
	mov	w13, #1                         ; =0x1
	fmov	d0, #1.00000000
Lloh4:
	adrp	x14, lCPI2_0@PAGE
Lloh5:
	ldr	d1, [x14, lCPI2_0@PAGEOFF]
	mov	w14, #8                         ; =0x8
	dup.2d	v2, x14
Lloh6:
	adrp	x14, lCPI2_1@PAGE
Lloh7:
	ldr	q3, [x14, lCPI2_1@PAGEOFF]
	mov	w14, #2                         ; =0x2
	movi.2s	v4, #1
	movi.2s	v5, #3
	movi.2s	v6, #5
	movi.2s	v7, #7
	fmov.2d	v16, #1.00000000
	movi.2s	v17, #8
	b	LBB2_3
LBB2_2:                                 ;   in Loop: Header=BB2_3 Depth=1
	add	x8, x8, #1
	add	w13, w13, #1
	add	w14, w14, #2
	cmp	x8, x9
	b.eq	LBB2_12
LBB2_3:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB2_10 Depth 2
                                        ;     Child Loop BB2_8 Depth 2
	str	xzr, [x2, x8, lsl #3]
	movi	d18, #0000000000000000
	cmp	w0, #8
	b.lo	LBB2_6
; %bb.4:                                ;   in Loop: Header=BB2_3 Depth=1
	add	x15, x2, x8, lsl #3
	cmp	x15, x10
	b.hs	LBB2_9
; %bb.5:                                ;   in Loop: Header=BB2_3 Depth=1
	add	x15, x15, #8
	cmp	x15, x1
	b.ls	LBB2_9
LBB2_6:                                 ;   in Loop: Header=BB2_3 Depth=1
	mov	x15, #0                         ; =0x0
LBB2_7:                                 ;   in Loop: Header=BB2_3 Depth=1
	add	w16, w13, w15
	add	w17, w8, w15
	mul	w16, w16, w17
	add	w17, w14, w15, lsl #1
LBB2_8:                                 ;   Parent Loop BB2_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	mov	w3, w16
	lsr	x3, x3, #1
	add	w3, w15, w3
	add	w3, w3, #1
	scvtf	d19, w3
	fdiv	d19, d0, d19
	ldr	d20, [x1, x15, lsl #3]
	fmadd	d18, d19, d20, d18
	str	d18, [x2, x8, lsl #3]
	add	x3, x15, #1
	add	w16, w16, w17
	add	w17, w17, #2
	mov	x15, x3
	cmp	x9, x3
	b.ne	LBB2_8
	b	LBB2_2
LBB2_9:                                 ;   in Loop: Header=BB2_3 Depth=1
	dup.2d	v19, x8
	dup.2s	v20, w8
	add	x15, x8, #2
	dup.2d	v21, x15
	add	x15, x8, #4
	dup.2d	v22, x15
	add	x15, x8, #6
	dup.2d	v23, x15
	add	w15, w8, #2
	add	w16, w8, #4
	dup.2s	v24, w15
	add	w15, w8, #6
	dup.2s	v25, w16
	dup.2s	v26, w15
	mov	x15, x11
	mov	x16, x12
	mov.16b	v27, v3
	fmov	d28, d1
LBB2_10:                                ;   Parent Loop BB2_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	add.2d	v29, v27, v19
	add.2d	v30, v21, v27
	add.2d	v31, v22, v27
	add.2d	v8, v23, v27
	add.2s	v9, v28, v20
	add.2s	v10, v24, v28
	add.2s	v11, v25, v28
	add.2s	v12, v26, v28
	add.2s	v9, v9, v4
	add.2s	v10, v10, v4
	add.2s	v11, v11, v4
	add.2s	v12, v12, v4
	xtn.2s	v29, v29
	xtn.2s	v30, v30
	xtn.2s	v31, v31
	xtn.2s	v8, v8
	mul.2s	v29, v9, v29
	mul.2s	v30, v10, v30
	mul.2s	v31, v11, v31
	mul.2s	v8, v12, v8
	ushr.2s	v29, v29, #1
	xtn.2s	v9, v27
	fmov	d10, d9
	usra.2s	v10, v30, #1
	fmov	d30, d9
	usra.2s	v30, v31, #1
	mvn.8b	v31, v9
	usra.2s	v9, v8, #1
	sub.2s	v29, v29, v31
	add.2s	v31, v10, v5
	add.2s	v30, v30, v6
	add.2s	v8, v9, v7
	sshll.2d	v29, v29, #0
	scvtf.2d	v29, v29
	sshll.2d	v31, v31, #0
	scvtf.2d	v31, v31
	sshll.2d	v30, v30, #0
	scvtf.2d	v30, v30
	sshll.2d	v8, v8, #0
	scvtf.2d	v8, v8
	fdiv.2d	v29, v16, v29
	fdiv.2d	v31, v16, v31
	fdiv.2d	v30, v16, v30
	fdiv.2d	v8, v16, v8
	ldp	q9, q10, [x16, #-32]
	ldp	q11, q12, [x16], #64
	fmul.2d	v29, v29, v9
	mov	d9, v29[1]
	fmul.2d	v31, v31, v10
	mov	d10, v31[1]
	fmul.2d	v30, v30, v11
	mov	d11, v30[1]
	fmul.2d	v8, v8, v12
	mov	d12, v8[1]
	fadd	d18, d18, d29
	fadd	d18, d18, d9
	fadd	d18, d18, d31
	fadd	d18, d18, d10
	fadd	d18, d18, d30
	fadd	d18, d18, d11
	fadd	d18, d18, d8
	fadd	d18, d18, d12
	add.2d	v27, v27, v2
	add.2s	v28, v28, v17
	subs	x15, x15, #8
	b.ne	LBB2_10
; %bb.11:                               ;   in Loop: Header=BB2_3 Depth=1
	str	d18, [x2, x8, lsl #3]
	mov	x15, x11
	cmp	x11, x9
	b.eq	LBB2_2
	b	LBB2_7
LBB2_12:
	ldp	d9, d8, [sp, #32]               ; 16-byte Folded Reload
	ldp	d11, d10, [sp, #16]             ; 16-byte Folded Reload
	ldp	d13, d12, [sp], #48             ; 16-byte Folded Reload
LBB2_13:
	ret
	.loh AdrpLdr	Lloh6, Lloh7
	.loh AdrpLdr	Lloh4, Lloh5
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__literal8,8byte_literals
	.p2align	3, 0x0                          ; -- Begin function mul_AtAv
lCPI3_0:
	.long	0                               ; 0x0
	.long	1                               ; 0x1
	.section	__TEXT,__literal16,16byte_literals
	.p2align	4, 0x0
lCPI3_1:
	.quad	0                               ; 0x0
	.quad	1                               ; 0x1
	.section	__TEXT,__text,regular,pure_instructions
	.globl	_mul_AtAv
	.p2align	2
_mul_AtAv:                              ; @mul_AtAv
	.cfi_startproc
; %bb.0:
	cmp	w0, #1
	b.lt	LBB3_23
; %bb.1:
	stp	d11, d10, [sp, #-32]!           ; 16-byte Folded Spill
	stp	d9, d8, [sp, #16]               ; 16-byte Folded Spill
	.cfi_def_cfa_offset 32
	.cfi_offset b8, -8
	.cfi_offset b9, -16
	.cfi_offset b10, -24
	.cfi_offset b11, -32
	mov	x10, #0                         ; =0x0
	mov	w8, w0
	add	x9, x1, w0, uxtw #3
	and	x11, x8, #0xfffffff8
	add	x12, x1, #32
	fmov	d1, #1.00000000
	adrp	x13, lCPI3_0@PAGE
	ldr	d2, [x13, lCPI3_0@PAGEOFF]
	adrp	x14, lCPI3_1@PAGE
	ldr	q3, [x14, lCPI3_1@PAGEOFF]
	fmov.2d	v0, #1.00000000
	mov	w15, #8                         ; =0x8
	dup.2d	v4, x15
	movi.2s	v5, #8
	b	LBB3_3
LBB3_2:                                 ;   in Loop: Header=BB3_3 Depth=1
	mov	x10, x15
	cmp	x15, x8
	b.eq	LBB3_11
LBB3_3:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB3_9 Depth 2
                                        ;     Child Loop BB3_7 Depth 2
	str	xzr, [x3, x10, lsl #3]
	add	x15, x10, #1
	movi	d6, #0000000000000000
	cmp	w0, #8
	b.lo	LBB3_6
; %bb.4:                                ;   in Loop: Header=BB3_3 Depth=1
	add	x16, x3, x10, lsl #3
	cmp	x16, x9
	b.hs	LBB3_8
; %bb.5:                                ;   in Loop: Header=BB3_3 Depth=1
	add	x16, x16, #8
	cmp	x16, x1
	b.ls	LBB3_8
LBB3_6:                                 ;   in Loop: Header=BB3_3 Depth=1
	mov	x16, #0                         ; =0x0
LBB3_7:                                 ;   Parent Loop BB3_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	add	w17, w10, w16
	madd	w17, w17, w17, w17
	add	w17, w15, w17, lsr #1
	scvtf	d7, w17
	ldr	d16, [x1, x16, lsl #3]
	fdiv	d7, d1, d7
	fmadd	d6, d7, d16, d6
	str	d6, [x3, x10, lsl #3]
	add	x16, x16, #1
	cmp	x8, x16
	b.ne	LBB3_7
	b	LBB3_2
LBB3_8:                                 ;   in Loop: Header=BB3_3 Depth=1
	add	w16, w10, #1
	dup.2d	v7, x10
	dup.2s	v16, w16
	dup.2s	v17, w15
	add	x16, x10, #2
	dup.2d	v18, x16
	add	x16, x10, #4
	dup.2d	v19, x16
	add	x16, x10, #6
	dup.2d	v20, x16
	add	w16, w10, #3
	add	w17, w10, #5
	dup.2s	v21, w16
	add	w16, w10, #7
	dup.2s	v22, w17
	dup.2s	v23, w16
	mov	x16, x11
	mov	x17, x12
	mov.16b	v24, v3
	fmov	d25, d2
LBB3_9:                                 ;   Parent Loop BB3_3 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	add.2d	v26, v24, v7
	add.2d	v27, v18, v24
	add.2d	v28, v19, v24
	add.2d	v29, v20, v24
	add.2s	v30, v16, v25
	add.2s	v31, v21, v25
	add.2s	v8, v22, v25
	add.2s	v9, v23, v25
	xtn.2s	v26, v26
	xtn.2s	v27, v27
	xtn.2s	v28, v28
	xtn.2s	v29, v29
	mul.2s	v26, v30, v26
	mul.2s	v27, v31, v27
	mul.2s	v28, v8, v28
	mul.2s	v29, v9, v29
	fmov	d30, d17
	usra.2s	v30, v26, #1
	fmov	d26, d17
	usra.2s	v26, v27, #1
	fmov	d27, d17
	usra.2s	v27, v28, #1
	fmov	d28, d17
	usra.2s	v28, v29, #1
	sshll.2d	v29, v30, #0
	scvtf.2d	v29, v29
	sshll.2d	v26, v26, #0
	scvtf.2d	v26, v26
	sshll.2d	v27, v27, #0
	scvtf.2d	v27, v27
	sshll.2d	v28, v28, #0
	scvtf.2d	v28, v28
	fdiv.2d	v29, v0, v29
	fdiv.2d	v26, v0, v26
	fdiv.2d	v27, v0, v27
	fdiv.2d	v28, v0, v28
	ldp	q30, q31, [x17, #-32]
	ldp	q8, q9, [x17], #64
	fmul.2d	v29, v29, v30
	mov	d30, v29[1]
	fmul.2d	v26, v26, v31
	mov	d31, v26[1]
	fmul.2d	v27, v27, v8
	mov	d8, v27[1]
	fmul.2d	v28, v28, v9
	mov	d9, v28[1]
	fadd	d6, d6, d29
	fadd	d6, d6, d30
	fadd	d6, d6, d26
	fadd	d6, d6, d31
	fadd	d6, d6, d27
	fadd	d6, d6, d8
	fadd	d6, d6, d28
	fadd	d6, d6, d9
	add.2d	v24, v24, v4
	add.2s	v25, v25, v5
	subs	x16, x16, #8
	b.ne	LBB3_9
; %bb.10:                               ;   in Loop: Header=BB3_3 Depth=1
	str	d6, [x3, x10, lsl #3]
	mov	x16, x11
	cmp	x11, x8
	b.ne	LBB3_7
	b	LBB3_2
LBB3_11:
	mov	x9, #0                          ; =0x0
	and	x10, x8, #0xfffffff8
	add	x11, x3, #32
	mov	w12, #1                         ; =0x1
	fmov	d1, #1.00000000
	ldr	d2, [x13, lCPI3_0@PAGEOFF]
	mov	w13, #2                         ; =0x2
	ldr	q3, [x14, lCPI3_1@PAGEOFF]
	movi.2s	v4, #3
	add	x14, x3, x8, lsl #3
	movi.2s	v5, #5
	movi.2s	v6, #7
	mov	w15, #8                         ; =0x8
	dup.2d	v7, x15
	movi.2s	v16, #8
	b	LBB3_13
LBB3_12:                                ;   in Loop: Header=BB3_13 Depth=1
	add	x9, x9, #1
	add	w12, w12, #1
	add	w13, w13, #2
	cmp	x9, x8
	b.eq	LBB3_22
LBB3_13:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB3_20 Depth 2
                                        ;     Child Loop BB3_18 Depth 2
	str	xzr, [x2, x9, lsl #3]
	movi	d17, #0000000000000000
	cmp	w0, #8
	b.lo	LBB3_16
; %bb.14:                               ;   in Loop: Header=BB3_13 Depth=1
	add	x15, x2, x9, lsl #3
	cmp	x15, x14
	b.hs	LBB3_19
; %bb.15:                               ;   in Loop: Header=BB3_13 Depth=1
	add	x15, x15, #8
	cmp	x15, x3
	b.ls	LBB3_19
LBB3_16:                                ;   in Loop: Header=BB3_13 Depth=1
	mov	x15, #0                         ; =0x0
LBB3_17:                                ;   in Loop: Header=BB3_13 Depth=1
	add	w16, w12, w15
	add	w17, w9, w15
	mul	w16, w16, w17
	add	w17, w13, w15, lsl #1
LBB3_18:                                ;   Parent Loop BB3_13 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	mov	w1, w16
	lsr	x1, x1, #1
	add	w1, w15, w1
	add	w1, w1, #1
	scvtf	d18, w1
	fdiv	d18, d1, d18
	ldr	d19, [x3, x15, lsl #3]
	fmadd	d17, d18, d19, d17
	str	d17, [x2, x9, lsl #3]
	add	x1, x15, #1
	add	w16, w16, w17
	add	w17, w17, #2
	mov	x15, x1
	cmp	x8, x1
	b.ne	LBB3_18
	b	LBB3_12
LBB3_19:                                ;   in Loop: Header=BB3_13 Depth=1
	add	w15, w9, #1
	dup.2d	v18, x9
	dup.2s	v19, w15
	add	x15, x9, #2
	dup.2d	v20, x15
	add	x15, x9, #4
	dup.2d	v21, x15
	add	x15, x9, #6
	dup.2d	v22, x15
	add	w15, w9, #3
	dup.2s	v23, w15
	add	w15, w9, #5
	dup.2s	v24, w15
	add	w15, w9, #7
	dup.2s	v25, w15
	mov	x15, x10
	mov	x16, x11
	mov.16b	v26, v3
	fmov	d27, d2
LBB3_20:                                ;   Parent Loop BB3_13 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	add.2d	v28, v26, v18
	add.2d	v29, v20, v26
	add.2d	v30, v21, v26
	add.2d	v31, v22, v26
	add.2s	v8, v19, v27
	add.2s	v9, v23, v27
	add.2s	v10, v24, v27
	add.2s	v11, v25, v27
	xtn.2s	v28, v28
	xtn.2s	v29, v29
	xtn.2s	v30, v30
	xtn.2s	v31, v31
	mul.2s	v28, v8, v28
	mul.2s	v29, v9, v29
	mul.2s	v30, v10, v30
	mul.2s	v31, v11, v31
	ushr.2s	v28, v28, #1
	xtn.2s	v8, v26
	fmov	d9, d8
	usra.2s	v9, v29, #1
	fmov	d29, d8
	usra.2s	v29, v30, #1
	mvn.8b	v30, v8
	usra.2s	v8, v31, #1
	sub.2s	v28, v28, v30
	add.2s	v30, v9, v4
	add.2s	v29, v29, v5
	add.2s	v31, v8, v6
	sshll.2d	v28, v28, #0
	scvtf.2d	v28, v28
	sshll.2d	v30, v30, #0
	scvtf.2d	v30, v30
	sshll.2d	v29, v29, #0
	scvtf.2d	v29, v29
	sshll.2d	v31, v31, #0
	scvtf.2d	v31, v31
	fdiv.2d	v28, v0, v28
	fdiv.2d	v30, v0, v30
	fdiv.2d	v29, v0, v29
	fdiv.2d	v31, v0, v31
	ldp	q8, q9, [x16, #-32]
	ldp	q10, q11, [x16], #64
	fmul.2d	v28, v28, v8
	mov	d8, v28[1]
	fmul.2d	v30, v30, v9
	mov	d9, v30[1]
	fmul.2d	v29, v29, v10
	mov	d10, v29[1]
	fmul.2d	v31, v31, v11
	mov	d11, v31[1]
	fadd	d17, d17, d28
	fadd	d17, d17, d8
	fadd	d17, d17, d30
	fadd	d17, d17, d9
	fadd	d17, d17, d29
	fadd	d17, d17, d10
	fadd	d17, d17, d31
	fadd	d17, d17, d11
	add.2d	v26, v26, v7
	add.2s	v27, v27, v16
	subs	x15, x15, #8
	b.ne	LBB3_20
; %bb.21:                               ;   in Loop: Header=BB3_13 Depth=1
	str	d17, [x2, x9, lsl #3]
	mov	x15, x10
	cmp	x10, x8
	b.eq	LBB3_12
	b	LBB3_17
LBB3_22:
	ldp	d9, d8, [sp, #16]               ; 16-byte Folded Reload
	ldp	d11, d10, [sp], #32             ; 16-byte Folded Reload
LBB3_23:
	ret
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__literal8,8byte_literals
	.p2align	3, 0x0                          ; -- Begin function main
lCPI4_0:
	.long	0                               ; 0x0
	.long	1                               ; 0x1
	.section	__TEXT,__literal16,16byte_literals
	.p2align	4, 0x0
lCPI4_1:
	.quad	0                               ; 0x0
	.quad	1                               ; 0x1
	.section	__TEXT,__text,regular,pure_instructions
	.globl	_main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	cmp	w0, #2
	b.ge	LBB4_2
; %bb.1:
	mov	w0, #1                          ; =0x1
	ret
LBB4_2:
	sub	sp, sp, #128
	stp	d13, d12, [sp, #16]             ; 16-byte Folded Spill
	stp	d11, d10, [sp, #32]             ; 16-byte Folded Spill
	stp	d9, d8, [sp, #48]               ; 16-byte Folded Spill
	stp	x24, x23, [sp, #64]             ; 16-byte Folded Spill
	stp	x22, x21, [sp, #80]             ; 16-byte Folded Spill
	stp	x20, x19, [sp, #96]             ; 16-byte Folded Spill
	stp	x29, x30, [sp, #112]            ; 16-byte Folded Spill
	add	x29, sp, #112
	.cfi_def_cfa w29, 16
	.cfi_offset w30, -8
	.cfi_offset w29, -16
	.cfi_offset w19, -24
	.cfi_offset w20, -32
	.cfi_offset w21, -40
	.cfi_offset w22, -48
	.cfi_offset w23, -56
	.cfi_offset w24, -64
	.cfi_offset b8, -72
	.cfi_offset b9, -80
	.cfi_offset b10, -88
	.cfi_offset b11, -96
	.cfi_offset b12, -104
	.cfi_offset b13, -112
	ldr	x0, [x1, #8]
	bl	_atoi
	mov	x22, x0
	sbfiz	x21, x22, #3, #32
	mov	x0, x21
	bl	_malloc
	mov	x19, x0
	mov	x0, x21
	bl	_malloc
	mov	x20, x0
	mov	x0, x21
	bl	_malloc
	mov	x21, x0
	mov	w23, w22
	cmp	w22, #1
	b.lt	LBB4_4
; %bb.3:
	lsl	x2, x23, #3
Lloh8:
	adrp	x1, l_.memset_pattern@PAGE
Lloh9:
	add	x1, x1, l_.memset_pattern@PAGEOFF
	mov	x0, x19
	bl	_memset_pattern16
LBB4_4:
	mov	w8, #0                          ; =0x0
	and	x9, x23, #0xfffffff8
	add	x10, x19, #32
	fmov	d0, #1.00000000
	adrp	x11, lCPI4_0@PAGE
	ldr	d1, [x11, lCPI4_0@PAGEOFF]
	add	x12, x21, #32
	adrp	x13, lCPI4_1@PAGE
	ldr	q2, [x13, lCPI4_1@PAGEOFF]
	add	x14, x20, #32
	movi.2s	v3, #3
	movi.2s	v4, #5
	movi.2s	v5, #7
	fmov.2d	v6, #1.00000000
	mov	w15, #8                         ; =0x8
	dup.2d	v7, x15
	movi.2s	v16, #8
	b	LBB4_6
LBB4_5:                                 ;   in Loop: Header=BB4_6 Depth=1
	add	w8, w8, #1
	cmp	w8, #10
	b.eq	LBB4_42
LBB4_6:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB4_9 Depth 2
                                        ;       Child Loop BB4_12 Depth 3
                                        ;       Child Loop BB4_14 Depth 3
                                        ;     Child Loop BB4_17 Depth 2
                                        ;       Child Loop BB4_20 Depth 3
                                        ;       Child Loop BB4_23 Depth 3
                                        ;     Child Loop BB4_26 Depth 2
                                        ;       Child Loop BB4_29 Depth 3
                                        ;       Child Loop BB4_31 Depth 3
                                        ;     Child Loop BB4_34 Depth 2
                                        ;       Child Loop BB4_37 Depth 3
                                        ;       Child Loop BB4_40 Depth 3
	cmp	w23, #1
	b.lt	LBB4_41
; %bb.7:                                ;   in Loop: Header=BB4_6 Depth=1
	mov	x16, #0                         ; =0x0
	b	LBB4_9
LBB4_8:                                 ;   in Loop: Header=BB4_9 Depth=2
	str	d17, [x21, x16, lsl #3]
	mov	x16, x17
	cmp	x17, x23
	b.eq	LBB4_15
LBB4_9:                                 ;   Parent Loop BB4_6 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB4_12 Depth 3
                                        ;       Child Loop BB4_14 Depth 3
	add	x17, x16, #1
	cmp	w23, #8
	b.hs	LBB4_11
; %bb.10:                               ;   in Loop: Header=BB4_9 Depth=2
	mov	x0, #0                          ; =0x0
	movi	d17, #0000000000000000
	b	LBB4_14
LBB4_11:                                ;   in Loop: Header=BB4_9 Depth=2
	add	w0, w16, #1
	dup.2d	v18, x16
	dup.2s	v19, w0
	dup.2s	v20, w17
	add	x0, x16, #2
	dup.2d	v21, x0
	add	x0, x16, #4
	dup.2d	v22, x0
	add	x0, x16, #6
	dup.2d	v23, x0
	add	w0, w16, #3
	dup.2s	v24, w0
	add	w0, w16, #5
	dup.2s	v25, w0
	add	w0, w16, #7
	ldr	d26, [x11, lCPI4_0@PAGEOFF]
	dup.2s	v27, w0
	movi	d17, #0000000000000000
	mov	x0, x9
	mov	x1, x10
	ldr	q28, [x13, lCPI4_1@PAGEOFF]
LBB4_12:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_9 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	add.2d	v29, v28, v18
	add.2d	v30, v21, v28
	add.2d	v31, v22, v28
	add.2d	v8, v23, v28
	add.2s	v9, v19, v26
	add.2s	v10, v24, v26
	add.2s	v11, v25, v26
	add.2s	v12, v27, v26
	xtn.2s	v29, v29
	xtn.2s	v30, v30
	xtn.2s	v31, v31
	xtn.2s	v8, v8
	mul.2s	v29, v9, v29
	mul.2s	v30, v10, v30
	mul.2s	v31, v11, v31
	mul.2s	v8, v12, v8
	fmov	d9, d20
	usra.2s	v9, v29, #1
	fmov	d29, d20
	usra.2s	v29, v30, #1
	fmov	d30, d20
	usra.2s	v30, v31, #1
	fmov	d31, d20
	usra.2s	v31, v8, #1
	sshll.2d	v8, v9, #0
	scvtf.2d	v8, v8
	sshll.2d	v29, v29, #0
	scvtf.2d	v29, v29
	sshll.2d	v30, v30, #0
	scvtf.2d	v30, v30
	sshll.2d	v31, v31, #0
	scvtf.2d	v31, v31
	fdiv.2d	v8, v6, v8
	fdiv.2d	v29, v6, v29
	fdiv.2d	v30, v6, v30
	fdiv.2d	v31, v6, v31
	ldp	q9, q10, [x1, #-32]
	ldp	q11, q12, [x1], #64
	fmul.2d	v8, v8, v9
	mov	d9, v8[1]
	fmul.2d	v29, v29, v10
	mov	d10, v29[1]
	fmul.2d	v30, v30, v11
	mov	d11, v30[1]
	fmul.2d	v31, v31, v12
	mov	d12, v31[1]
	fadd	d17, d17, d8
	fadd	d17, d17, d9
	fadd	d17, d17, d29
	fadd	d17, d17, d10
	fadd	d17, d17, d30
	fadd	d17, d17, d11
	fadd	d17, d17, d31
	fadd	d17, d17, d12
	dup.2d	v29, x15
	add.2d	v28, v28, v29
	add.2s	v26, v26, v16
	subs	x0, x0, #8
	b.ne	LBB4_12
; %bb.13:                               ;   in Loop: Header=BB4_9 Depth=2
	mov	x0, x9
	cmp	x9, x23
	b.eq	LBB4_8
LBB4_14:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_9 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	add	w1, w16, w0
	madd	w1, w1, w1, w1
	add	w1, w17, w1, lsr #1
	scvtf	d18, w1
	fdiv	d18, d0, d18
	ldr	d19, [x19, x0, lsl #3]
	fmadd	d17, d18, d19, d17
	add	x0, x0, #1
	cmp	x23, x0
	b.ne	LBB4_14
	b	LBB4_8
LBB4_15:                                ;   in Loop: Header=BB4_6 Depth=1
	mov	x16, #0                         ; =0x0
	mov	w17, #1                         ; =0x1
	mov	w0, #2                          ; =0x2
	b	LBB4_17
LBB4_16:                                ;   in Loop: Header=BB4_17 Depth=2
	str	d17, [x20, x16, lsl #3]
	add	x16, x16, #1
	add	w17, w17, #1
	add	w0, w0, #2
	cmp	x16, x23
	b.eq	LBB4_24
LBB4_17:                                ;   Parent Loop BB4_6 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB4_20 Depth 3
                                        ;       Child Loop BB4_23 Depth 3
	cmp	w23, #8
	b.hs	LBB4_19
; %bb.18:                               ;   in Loop: Header=BB4_17 Depth=2
	mov	x1, #0                          ; =0x0
	movi	d17, #0000000000000000
	b	LBB4_22
LBB4_19:                                ;   in Loop: Header=BB4_17 Depth=2
	add	w1, w16, #1
	dup.2d	v18, x16
	dup.2s	v19, w1
	add	x1, x16, #2
	dup.2d	v20, x1
	add	x1, x16, #4
	dup.2d	v21, x1
	add	x1, x16, #6
	dup.2d	v22, x1
	add	w1, w16, #3
	dup.2s	v23, w1
	add	w1, w16, #5
	dup.2s	v24, w1
	add	w1, w16, #7
	ldr	d25, [x11, lCPI4_0@PAGEOFF]
	dup.2s	v26, w1
	movi	d17, #0000000000000000
	mov	x1, x9
	mov	x2, x12
	ldr	q27, [x13, lCPI4_1@PAGEOFF]
LBB4_20:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_17 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	add.2d	v28, v27, v18
	add.2d	v29, v20, v27
	add.2d	v30, v21, v27
	add.2d	v31, v22, v27
	add.2s	v8, v19, v25
	add.2s	v9, v23, v25
	add.2s	v10, v24, v25
	add.2s	v11, v26, v25
	xtn.2s	v28, v28
	xtn.2s	v29, v29
	xtn.2s	v30, v30
	xtn.2s	v31, v31
	mul.2s	v28, v8, v28
	mul.2s	v29, v9, v29
	mul.2s	v30, v10, v30
	mul.2s	v31, v11, v31
	ushr.2s	v28, v28, #1
	xtn.2s	v8, v27
	fmov	d9, d8
	usra.2s	v9, v29, #1
	fmov	d29, d8
	usra.2s	v29, v30, #1
	mvn.8b	v30, v8
	usra.2s	v8, v31, #1
	sub.2s	v28, v28, v30
	add.2s	v30, v9, v3
	add.2s	v29, v29, v4
	add.2s	v31, v8, v5
	sshll.2d	v28, v28, #0
	scvtf.2d	v28, v28
	sshll.2d	v30, v30, #0
	scvtf.2d	v30, v30
	sshll.2d	v29, v29, #0
	scvtf.2d	v29, v29
	sshll.2d	v31, v31, #0
	scvtf.2d	v31, v31
	fdiv.2d	v28, v6, v28
	fdiv.2d	v30, v6, v30
	fdiv.2d	v29, v6, v29
	fdiv.2d	v31, v6, v31
	ldp	q8, q9, [x2, #-32]
	ldp	q10, q11, [x2], #64
	fmul.2d	v28, v28, v8
	mov	d8, v28[1]
	fmul.2d	v30, v30, v9
	mov	d9, v30[1]
	fmul.2d	v29, v29, v10
	mov	d10, v29[1]
	fmul.2d	v31, v31, v11
	mov	d11, v31[1]
	fadd	d17, d17, d28
	fadd	d17, d17, d8
	fadd	d17, d17, d30
	fadd	d17, d17, d9
	fadd	d17, d17, d29
	fadd	d17, d17, d10
	fadd	d17, d17, d31
	fadd	d17, d17, d11
	dup.2d	v28, x15
	add.2d	v27, v27, v28
	add.2s	v25, v25, v16
	subs	x1, x1, #8
	b.ne	LBB4_20
; %bb.21:                               ;   in Loop: Header=BB4_17 Depth=2
	mov	x1, x9
	cmp	x9, x23
	b.eq	LBB4_16
LBB4_22:                                ;   in Loop: Header=BB4_17 Depth=2
	add	w2, w17, w1
	add	w3, w16, w1
	mul	w2, w2, w3
	add	w3, w0, w1, lsl #1
LBB4_23:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_17 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	mov	w4, w2
	lsr	x4, x4, #1
	ldr	d18, [x21, x1, lsl #3]
	add	w4, w1, w4
	add	x5, x1, #1
	add	w1, w4, #1
	scvtf	d19, w1
	fdiv	d19, d0, d19
	fmadd	d17, d19, d18, d17
	add	w2, w2, w3
	add	w3, w3, #2
	mov	x1, x5
	cmp	x23, x5
	b.ne	LBB4_23
	b	LBB4_16
LBB4_24:                                ;   in Loop: Header=BB4_6 Depth=1
	mov	x16, #0                         ; =0x0
	b	LBB4_26
LBB4_25:                                ;   in Loop: Header=BB4_26 Depth=2
	str	d17, [x21, x16, lsl #3]
	mov	x16, x17
	cmp	x17, x23
	b.eq	LBB4_32
LBB4_26:                                ;   Parent Loop BB4_6 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB4_29 Depth 3
                                        ;       Child Loop BB4_31 Depth 3
	add	x17, x16, #1
	cmp	w23, #8
	b.hs	LBB4_28
; %bb.27:                               ;   in Loop: Header=BB4_26 Depth=2
	mov	x0, #0                          ; =0x0
	movi	d17, #0000000000000000
	b	LBB4_31
LBB4_28:                                ;   in Loop: Header=BB4_26 Depth=2
	add	w0, w16, #1
	dup.2d	v18, x16
	dup.2s	v19, w0
	dup.2s	v20, w17
	add	x0, x16, #2
	dup.2d	v21, x0
	add	x0, x16, #4
	dup.2d	v22, x0
	add	x0, x16, #6
	dup.2d	v23, x0
	add	w0, w16, #3
	add	w1, w16, #5
	dup.2s	v24, w0
	add	w0, w16, #7
	dup.2s	v25, w1
	movi	d17, #0000000000000000
	dup.2s	v26, w0
	mov	x0, x9
	mov	x1, x14
	mov.16b	v27, v2
	fmov	d28, d1
LBB4_29:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_26 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	add.2d	v29, v27, v18
	add.2d	v30, v21, v27
	add.2d	v31, v22, v27
	add.2d	v8, v23, v27
	add.2s	v9, v19, v28
	add.2s	v10, v24, v28
	add.2s	v11, v25, v28
	add.2s	v12, v26, v28
	xtn.2s	v29, v29
	xtn.2s	v30, v30
	xtn.2s	v31, v31
	xtn.2s	v8, v8
	mul.2s	v29, v9, v29
	mul.2s	v30, v10, v30
	mul.2s	v31, v11, v31
	mul.2s	v8, v12, v8
	fmov	d9, d20
	usra.2s	v9, v29, #1
	fmov	d29, d20
	usra.2s	v29, v30, #1
	fmov	d30, d20
	usra.2s	v30, v31, #1
	fmov	d31, d20
	usra.2s	v31, v8, #1
	sshll.2d	v8, v9, #0
	scvtf.2d	v8, v8
	sshll.2d	v29, v29, #0
	scvtf.2d	v29, v29
	sshll.2d	v30, v30, #0
	scvtf.2d	v30, v30
	sshll.2d	v31, v31, #0
	scvtf.2d	v31, v31
	fdiv.2d	v8, v6, v8
	fdiv.2d	v29, v6, v29
	fdiv.2d	v30, v6, v30
	fdiv.2d	v31, v6, v31
	ldp	q9, q10, [x1, #-32]
	ldp	q11, q12, [x1], #64
	fmul.2d	v8, v8, v9
	mov	d9, v8[1]
	fmul.2d	v29, v29, v10
	mov	d10, v29[1]
	fmul.2d	v30, v30, v11
	mov	d11, v30[1]
	fmul.2d	v31, v31, v12
	mov	d12, v31[1]
	fadd	d17, d17, d8
	fadd	d17, d17, d9
	fadd	d17, d17, d29
	fadd	d17, d17, d10
	fadd	d17, d17, d30
	fadd	d17, d17, d11
	fadd	d17, d17, d31
	fadd	d17, d17, d12
	dup.2d	v29, x15
	add.2d	v27, v27, v29
	add.2s	v28, v28, v16
	subs	x0, x0, #8
	b.ne	LBB4_29
; %bb.30:                               ;   in Loop: Header=BB4_26 Depth=2
	mov	x0, x9
	cmp	x9, x23
	b.eq	LBB4_25
LBB4_31:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_26 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	add	w1, w16, w0
	madd	w1, w1, w1, w1
	add	w1, w17, w1, lsr #1
	scvtf	d18, w1
	fdiv	d18, d0, d18
	ldr	d19, [x20, x0, lsl #3]
	fmadd	d17, d18, d19, d17
	add	x0, x0, #1
	cmp	x23, x0
	b.ne	LBB4_31
	b	LBB4_25
LBB4_32:                                ;   in Loop: Header=BB4_6 Depth=1
	mov	x16, #0                         ; =0x0
	mov	w17, #1                         ; =0x1
	mov	w0, #2                          ; =0x2
	b	LBB4_34
LBB4_33:                                ;   in Loop: Header=BB4_34 Depth=2
	str	d17, [x19, x16, lsl #3]
	add	x16, x16, #1
	add	w17, w17, #1
	add	w0, w0, #2
	cmp	x16, x23
	b.eq	LBB4_5
LBB4_34:                                ;   Parent Loop BB4_6 Depth=1
                                        ; =>  This Loop Header: Depth=2
                                        ;       Child Loop BB4_37 Depth 3
                                        ;       Child Loop BB4_40 Depth 3
	cmp	w23, #8
	b.hs	LBB4_36
; %bb.35:                               ;   in Loop: Header=BB4_34 Depth=2
	mov	x1, #0                          ; =0x0
	movi	d17, #0000000000000000
	b	LBB4_39
LBB4_36:                                ;   in Loop: Header=BB4_34 Depth=2
	add	w1, w16, #1
	dup.2d	v18, x16
	dup.2s	v19, w1
	add	x1, x16, #2
	dup.2d	v20, x1
	add	x1, x16, #4
	dup.2d	v21, x1
	add	x1, x16, #6
	dup.2d	v22, x1
	add	w1, w16, #3
	dup.2s	v23, w1
	add	w1, w16, #5
	dup.2s	v24, w1
	add	w1, w16, #7
	dup.2s	v25, w1
	movi	d17, #0000000000000000
	mov	x1, x9
	mov	x2, x12
	mov.16b	v26, v2
	fmov	d27, d1
LBB4_37:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_34 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	add.2d	v28, v26, v18
	add.2d	v29, v20, v26
	add.2d	v30, v21, v26
	add.2d	v31, v22, v26
	add.2s	v8, v19, v27
	add.2s	v9, v23, v27
	add.2s	v10, v24, v27
	add.2s	v11, v25, v27
	xtn.2s	v28, v28
	xtn.2s	v29, v29
	xtn.2s	v30, v30
	xtn.2s	v31, v31
	mul.2s	v28, v8, v28
	mul.2s	v29, v9, v29
	mul.2s	v30, v10, v30
	mul.2s	v31, v11, v31
	ushr.2s	v28, v28, #1
	xtn.2s	v8, v26
	fmov	d9, d8
	usra.2s	v9, v29, #1
	fmov	d29, d8
	usra.2s	v29, v30, #1
	mvn.8b	v30, v8
	usra.2s	v8, v31, #1
	sub.2s	v28, v28, v30
	add.2s	v30, v9, v3
	add.2s	v29, v29, v4
	add.2s	v31, v8, v5
	sshll.2d	v28, v28, #0
	scvtf.2d	v28, v28
	sshll.2d	v30, v30, #0
	scvtf.2d	v30, v30
	sshll.2d	v29, v29, #0
	scvtf.2d	v29, v29
	sshll.2d	v31, v31, #0
	scvtf.2d	v31, v31
	fdiv.2d	v28, v6, v28
	fdiv.2d	v30, v6, v30
	fdiv.2d	v29, v6, v29
	fdiv.2d	v31, v6, v31
	ldp	q8, q9, [x2, #-32]
	ldp	q10, q11, [x2], #64
	fmul.2d	v28, v28, v8
	mov	d8, v28[1]
	fmul.2d	v30, v30, v9
	mov	d9, v30[1]
	fmul.2d	v29, v29, v10
	mov	d10, v29[1]
	fmul.2d	v31, v31, v11
	mov	d11, v31[1]
	fadd	d17, d17, d28
	fadd	d17, d17, d8
	fadd	d17, d17, d30
	fadd	d17, d17, d9
	fadd	d17, d17, d29
	fadd	d17, d17, d10
	fadd	d17, d17, d31
	fadd	d17, d17, d11
	add.2d	v26, v26, v7
	add.2s	v27, v27, v16
	subs	x1, x1, #8
	b.ne	LBB4_37
; %bb.38:                               ;   in Loop: Header=BB4_34 Depth=2
	mov	x1, x9
	cmp	x9, x23
	b.eq	LBB4_33
LBB4_39:                                ;   in Loop: Header=BB4_34 Depth=2
	add	w2, w17, w1
	add	w3, w16, w1
	mul	w2, w2, w3
	add	w3, w0, w1, lsl #1
LBB4_40:                                ;   Parent Loop BB4_6 Depth=1
                                        ;     Parent Loop BB4_34 Depth=2
                                        ; =>    This Inner Loop Header: Depth=3
	mov	w4, w2
	lsr	x4, x4, #1
	ldr	d18, [x21, x1, lsl #3]
	add	w4, w1, w4
	add	x5, x1, #1
	add	w1, w4, #1
	scvtf	d19, w1
	fdiv	d19, d0, d19
	fmadd	d17, d19, d18, d17
	add	w2, w2, w3
	add	w3, w3, #2
	mov	x1, x5
	cmp	x23, x5
	b.ne	LBB4_40
	b	LBB4_33
LBB4_41:                                ;   in Loop: Header=BB4_6 Depth=1
	add	w8, w8, #1
	cmp	w8, #10
	b.ne	LBB4_6
	b	LBB4_45
LBB4_42:
	cmp	w23, #1
	b.lt	LBB4_45
; %bb.43:
	cmp	w23, #4
	b.hs	LBB4_46
; %bb.44:
	mov	x8, #0                          ; =0x0
	movi	d0, #0000000000000000
	movi	d1, #0000000000000000
	b	LBB4_49
LBB4_45:
	mov	x8, #9221120237041090560        ; =0x7ff8000000000000
	fmov	d0, x8
	b	LBB4_52
LBB4_46:
	and	x8, x23, #0xfffffffc
	add	x9, x20, #16
	add	x10, x19, #16
	movi	d0, #0000000000000000
	mov	x11, x8
	movi	d1, #0000000000000000
LBB4_47:                                ; =>This Inner Loop Header: Depth=1
	ldp	d2, d3, [x10, #-16]
	ldp	d4, d5, [x10], #32
	ldp	d6, d7, [x9, #-16]
	ldp	d16, d17, [x9], #32
	fmul	d2, d2, d6
	fmul	d3, d3, d7
	fmul	d4, d4, d16
	fmul	d5, d5, d17
	fmul	d6, d6, d6
	fmul	d7, d7, d7
	fmul	d16, d16, d16
	fmul	d17, d17, d17
	fadd	d0, d0, d6
	fadd	d0, d0, d7
	fadd	d0, d0, d16
	fadd	d0, d0, d17
	fadd	d1, d1, d2
	fadd	d1, d1, d3
	fadd	d1, d1, d4
	fadd	d1, d1, d5
	subs	x11, x11, #4
	b.ne	LBB4_47
; %bb.48:
	cmp	x8, x23
	b.eq	LBB4_51
LBB4_49:
	lsl	x10, x8, #3
	add	x9, x20, x10
	add	x10, x19, x10
	sub	x8, x23, x8
LBB4_50:                                ; =>This Inner Loop Header: Depth=1
	ldr	d2, [x10], #8
	ldr	d3, [x9], #8
	fmadd	d1, d2, d3, d1
	fmadd	d0, d3, d3, d0
	subs	x8, x8, #1
	b.ne	LBB4_50
LBB4_51:
	fdiv	d0, d1, d0
LBB4_52:
	fsqrt	d0, d0
	str	d0, [sp]
Lloh10:
	adrp	x0, l_.str@PAGE
Lloh11:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	x0, x19
	bl	_free
	mov	x0, x20
	bl	_free
	mov	x0, x21
	bl	_free
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #112]            ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #96]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #80]             ; 16-byte Folded Reload
	ldp	x24, x23, [sp, #64]             ; 16-byte Folded Reload
	ldp	d9, d8, [sp, #48]               ; 16-byte Folded Reload
	ldp	d11, d10, [sp, #32]             ; 16-byte Folded Reload
	ldp	d13, d12, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #128
	ret
	.loh AdrpAdd	Lloh8, Lloh9
	.loh AdrpAdd	Lloh10, Lloh11
	.cfi_endproc
                                        ; -- End function
	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%.9f\n"

	.section	__TEXT,__literal16,16byte_literals
	.p2align	4, 0x0                          ; @.memset_pattern
l_.memset_pattern:
	.quad	0x3ff0000000000000              ; double 1
	.quad	0x3ff0000000000000              ; double 1

.subsections_via_symbols

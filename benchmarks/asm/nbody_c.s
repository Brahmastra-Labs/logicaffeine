	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_offset_momentum                ; -- Begin function offset_momentum
	.p2align	2
_offset_momentum:                       ; @offset_momentum
	.cfi_startproc
; %bb.0:
Lloh0:
	adrp	x8, _bodies@PAGE+24
Lloh1:
	add	x8, x8, _bodies@PAGEOFF+24
	ldp	d1, d0, [x8, #16]
	movi	d2, #0000000000000000
	fmadd	d1, d1, d0, d2
	ldp	d3, d2, [x8, #72]
	fmadd	d1, d3, d2, d1
	ldp	d4, d3, [x8, #128]
	fmadd	d1, d4, d3, d1
	ldp	d5, d4, [x8, #184]
	fmadd	d1, d5, d4, d1
	ldp	d6, d5, [x8, #240]
	fmadd	d1, d6, d5, d1
	ldr	q6, [x8]
	movi.2d	v7, #0000000000000000
	fmla.2d	v7, v6, v0[0]
	ldur	q0, [x8, #56]
	fmla.2d	v7, v0, v2[0]
	ldr	q0, [x8, #112]
	fmla.2d	v7, v0, v3[0]
	ldur	q0, [x8, #168]
	fmla.2d	v7, v0, v4[0]
	ldr	q0, [x8, #224]
	mov	x9, #17886                      ; =0x45de
	movk	x9, #51646, lsl #16
	movk	x9, #48444, lsl #32
	movk	x9, #49219, lsl #48
	dup.2d	v2, x9
	fmla.2d	v7, v0, v5[0]
	fdiv.2d	v0, v7, v2
	str	q0, [x8]
	fmov	d0, x9
	fdiv	d0, d1, d0
	str	d0, [x8, #16]
	ret
	.loh AdrpAdd	Lloh0, Lloh1
	.cfi_endproc
                                        ; -- End function
	.globl	_energy                         ; -- Begin function energy
	.p2align	2
_energy:                                ; @energy
	.cfi_startproc
; %bb.0:
	mov	x8, #0                          ; =0x0
Lloh2:
	adrp	x9, _bodies@PAGE+56
Lloh3:
	add	x9, x9, _bodies@PAGEOFF+56
	movi	d0, #0000000000000000
	mov	w10, #4                         ; =0x4
	mov	w11, #56                        ; =0x38
	fmov	d1, #0.50000000
Lloh4:
	adrp	x12, _bodies@PAGE
Lloh5:
	add	x12, x12, _bodies@PAGEOFF
	b	LBB1_2
LBB1_1:                                 ;   in Loop: Header=BB1_2 Depth=1
	add	x8, x8, #1
	sub	x10, x10, #1
	add	x9, x9, #56
	cmp	x8, #5
	b.eq	LBB1_5
LBB1_2:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB1_4 Depth 2
	madd	x13, x8, x11, x12
	ldp	d3, d2, [x13, #40]
	fmul	d4, d2, d1
	ldp	d5, d6, [x13, #24]
	fmul	d6, d6, d6
	fmadd	d5, d5, d5, d6
	fmadd	d3, d3, d3, d5
	fmadd	d0, d4, d3, d0
	cmp	x8, #3
	b.hi	LBB1_1
; %bb.3:                                ;   in Loop: Header=BB1_2 Depth=1
	madd	x13, x8, x11, x12
	ldp	d3, d4, [x13]
	ldr	d5, [x13, #16]
	mov	x13, x9
	mov	x14, x10
LBB1_4:                                 ;   Parent Loop BB1_2 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldp	d6, d7, [x13]
	fsub	d7, d4, d7
	fsub	d6, d3, d6
	ldr	d16, [x13, #16]
	fsub	d16, d5, d16
	ldr	d17, [x13, #48]
	fmul	d7, d7, d7
	fmadd	d6, d6, d6, d7
	fmul	d7, d2, d17
	fmadd	d6, d16, d16, d6
	fsqrt	d6, d6
	fdiv	d6, d7, d6
	fsub	d0, d0, d6
	add	x13, x13, #56
	subs	x14, x14, #1
	b.ne	LBB1_4
	b	LBB1_1
LBB1_5:
	ret
	.loh AdrpAdd	Lloh4, Lloh5
	.loh AdrpAdd	Lloh2, Lloh3
	.cfi_endproc
                                        ; -- End function
	.globl	_advance                        ; -- Begin function advance
	.p2align	2
_advance:                               ; @advance
	.cfi_startproc
; %bb.0:
                                        ; kill: def $d0 killed $d0 def $q0
	mov	x9, #0                          ; =0x0
Lloh6:
	adrp	x10, _bodies@PAGE+104
Lloh7:
	add	x10, x10, _bodies@PAGEOFF+104
	mov	w11, #4                         ; =0x4
	mov	w12, #56                        ; =0x38
Lloh8:
	adrp	x8, _bodies@PAGE
Lloh9:
	add	x8, x8, _bodies@PAGEOFF
	b	LBB2_2
LBB2_1:                                 ;   in Loop: Header=BB2_2 Depth=1
	add	x9, x9, #1
	add	x10, x10, #56
	sub	x11, x11, #1
	cmp	x9, #5
	b.eq	LBB2_5
LBB2_2:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB2_4 Depth 2
	cmp	x9, #3
	b.hi	LBB2_1
; %bb.3:                                ;   in Loop: Header=BB2_2 Depth=1
	madd	x15, x9, x12, x8
	ldr	q1, [x15]
	ldr	d2, [x15, #16]
	add	x13, x15, #24
	add	x14, x15, #40
	ldr	d3, [x15, #48]
	mov	x15, x11
	mov	x16, x10
LBB2_4:                                 ;   Parent Loop BB2_2 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldur	d4, [x16, #-32]
	fsub	d4, d2, d4
	ldr	d5, [x16]
	ldr	d6, [x14]
	fnmul	d7, d4, d5
	ldur	q16, [x16, #-48]
	fsub.2d	v16, v1, v16
	fmul.2d	v17, v16, v16
	mov	d17, v17[1]
	fmla.d	d17, d16, v16[0]
	fmadd	d17, d4, d4, d17
	fsqrt	d17, d17
	fmul	d18, d17, d17
	fmul	d17, d17, d18
	fdiv	d17, d0, d17
	ldr	q18, [x13]
	fneg.2d	v19, v16
	fmul.2d	v5, v19, v5[0]
	fmla.2d	v18, v5, v17[0]
	str	q18, [x13]
	fmadd	d5, d7, d17, d6
	str	d5, [x14]
	fmul.2d	v5, v16, v3[0]
	ldur	q6, [x16, #-24]
	fmla.2d	v6, v5, v17[0]
	stur	q6, [x16, #-24]
	fmul	d4, d4, d3
	ldur	d5, [x16, #-8]
	fmadd	d4, d4, d17, d5
	stur	d4, [x16, #-8]
	add	x16, x16, #56
	subs	x15, x15, #1
	b.ne	LBB2_4
	b	LBB2_1
LBB2_5:
	ldur	q1, [x8, #24]
	ldr	q2, [x8]
	fmla.2d	v2, v1, v0[0]
	str	q2, [x8]
	ldr	d1, [x8, #40]
	ldr	d2, [x8, #16]
	fmadd	d1, d0, d1, d2
	str	d1, [x8, #16]
	ldr	q1, [x8, #80]
	ldur	q2, [x8, #56]
	fmla.2d	v2, v1, v0[0]
	stur	q2, [x8, #56]
	ldr	d1, [x8, #96]
	ldr	d2, [x8, #72]
	fmadd	d1, d0, d1, d2
	str	d1, [x8, #72]
	ldur	q1, [x8, #136]
	ldr	q2, [x8, #112]
	fmla.2d	v2, v1, v0[0]
	str	q2, [x8, #112]
	ldr	d1, [x8, #152]
	ldr	d2, [x8, #128]
	fmadd	d1, d0, d1, d2
	str	d1, [x8, #128]
	ldr	q1, [x8, #192]
	ldur	q2, [x8, #168]
	fmla.2d	v2, v1, v0[0]
	stur	q2, [x8, #168]
	ldr	d1, [x8, #208]
	ldr	d2, [x8, #184]
	fmadd	d1, d0, d1, d2
	str	d1, [x8, #184]
	ldur	q1, [x8, #248]
	ldr	q2, [x8, #224]
	fmla.2d	v2, v1, v0[0]
	str	q2, [x8, #224]
	ldr	d1, [x8, #264]
	ldr	d2, [x8, #240]
	fmadd	d0, d0, d1, d2
	str	d0, [x8, #240]
	ret
	.loh AdrpAdd	Lloh8, Lloh9
	.loh AdrpAdd	Lloh6, Lloh7
	.cfi_endproc
                                        ; -- End function
	.globl	_main                           ; -- Begin function main
	.p2align	2
_main:                                  ; @main
	.cfi_startproc
; %bb.0:
	cmp	w0, #2
	b.ge	LBB3_2
; %bb.1:
	mov	w0, #1                          ; =0x1
	ret
LBB3_2:
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
	mov	x19, x0
	mov	x8, #0                          ; =0x0
Lloh10:
	adrp	x10, _bodies@PAGE+24
Lloh11:
	add	x10, x10, _bodies@PAGEOFF+24
	ldp	d2, d1, [x10, #16]
	movi	d0, #0000000000000000
	fmadd	d2, d2, d1, d0
	add	x9, x10, #32
	ldp	d4, d3, [x10, #72]
	fmadd	d2, d4, d3, d2
	ldp	d5, d4, [x10, #128]
	fmadd	d2, d5, d4, d2
	ldp	d6, d5, [x10, #184]
	fmadd	d2, d6, d5, d2
	ldp	d7, d6, [x10, #240]
	fmadd	d2, d7, d6, d2
	ldr	q7, [x10]
	movi.2d	v16, #0000000000000000
	fmla.2d	v16, v7, v1[0]
	ldur	q1, [x10, #56]
	fmla.2d	v16, v1, v3[0]
	ldr	q1, [x10, #112]
	fmla.2d	v16, v1, v4[0]
	ldur	q1, [x10, #168]
	fmla.2d	v16, v1, v5[0]
	ldr	q1, [x10, #224]
	fmla.2d	v16, v1, v6[0]
	mov	x11, #17886                     ; =0x45de
	movk	x11, #51646, lsl #16
	movk	x11, #48444, lsl #32
	movk	x11, #49219, lsl #48
	dup.2d	v1, x11
	fdiv.2d	v1, v16, v1
	str	q1, [x10]
	fmov	d1, x11
	fdiv	d1, d2, d1
	str	d1, [x10, #16]
	mov	w10, #4                         ; =0x4
	mov	w11, #56                        ; =0x38
Lloh12:
	adrp	x20, _bodies@PAGE
Lloh13:
	add	x20, x20, _bodies@PAGEOFF
	fmov	d1, #0.50000000
	b	LBB3_4
LBB3_3:                                 ;   in Loop: Header=BB3_4 Depth=1
	add	x8, x8, #1
	sub	x10, x10, #1
	add	x9, x9, #56
	cmp	x8, #5
	b.eq	LBB3_7
LBB3_4:                                 ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB3_6 Depth 2
	madd	x12, x8, x11, x20
	ldp	d3, d2, [x12, #40]
	fmul	d4, d2, d1
	ldp	d5, d6, [x12, #24]
	fmul	d6, d6, d6
	fmadd	d5, d5, d5, d6
	fmadd	d3, d3, d3, d5
	fmadd	d0, d4, d3, d0
	cmp	x8, #3
	b.hi	LBB3_3
; %bb.5:                                ;   in Loop: Header=BB3_4 Depth=1
	madd	x12, x8, x11, x20
	ldp	d3, d4, [x12]
	ldr	d5, [x12, #16]
	mov	x12, x9
	mov	x13, x10
LBB3_6:                                 ;   Parent Loop BB3_4 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldp	d6, d7, [x12]
	fsub	d7, d4, d7
	fsub	d6, d3, d6
	ldr	d16, [x12, #16]
	fsub	d16, d5, d16
	ldr	d17, [x12, #48]
	fmul	d7, d7, d7
	fmadd	d6, d6, d6, d7
	fmul	d7, d2, d17
	fmadd	d6, d16, d16, d6
	fsqrt	d6, d6
	fdiv	d6, d7, d6
	fsub	d0, d0, d6
	add	x12, x12, #56
	subs	x13, x13, #1
	b.ne	LBB3_6
	b	LBB3_3
LBB3_7:
	str	d0, [sp]
Lloh14:
	adrp	x0, l_.str@PAGE
Lloh15:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	cmp	x19, #1
	b.lt	LBB3_10
; %bb.8:
	mov	x21, #5243                      ; =0x147b
	movk	x21, #18350, lsl #16
	movk	x21, #31457, lsl #32
	movk	x21, #16260, lsl #48
LBB3_9:                                 ; =>This Inner Loop Header: Depth=1
	fmov	d0, x21
	bl	_advance
	subs	x19, x19, #1
	b.ne	LBB3_9
LBB3_10:
	mov	x8, #0                          ; =0x0
	movi	d0, #0000000000000000
	mov	w9, #4                          ; =0x4
	mov	w10, #56                        ; =0x38
	fmov	d1, #0.50000000
Lloh16:
	adrp	x11, _bodies@PAGE+56
Lloh17:
	add	x11, x11, _bodies@PAGEOFF+56
	b	LBB3_12
LBB3_11:                                ;   in Loop: Header=BB3_12 Depth=1
	add	x8, x8, #1
	sub	x9, x9, #1
	add	x11, x11, #56
	cmp	x8, #5
	b.eq	LBB3_15
LBB3_12:                                ; =>This Loop Header: Depth=1
                                        ;     Child Loop BB3_14 Depth 2
	madd	x12, x8, x10, x20
	ldp	d3, d2, [x12, #40]
	fmul	d4, d2, d1
	ldp	d5, d6, [x12, #24]
	fmul	d6, d6, d6
	fmadd	d5, d5, d5, d6
	fmadd	d3, d3, d3, d5
	fmadd	d0, d4, d3, d0
	cmp	x8, #3
	b.hi	LBB3_11
; %bb.13:                               ;   in Loop: Header=BB3_12 Depth=1
	madd	x12, x8, x10, x20
	ldp	d3, d4, [x12]
	ldr	d5, [x12, #16]
	mov	x12, x11
	mov	x13, x9
LBB3_14:                                ;   Parent Loop BB3_12 Depth=1
                                        ; =>  This Inner Loop Header: Depth=2
	ldp	d6, d7, [x12]
	fsub	d7, d4, d7
	fsub	d6, d3, d6
	ldr	d16, [x12, #16]
	fsub	d16, d5, d16
	ldr	d17, [x12, #48]
	fmul	d7, d7, d7
	fmadd	d6, d6, d6, d7
	fmul	d7, d2, d17
	fmadd	d6, d16, d16, d6
	fsqrt	d6, d6
	fdiv	d6, d7, d6
	fsub	d0, d0, d6
	add	x12, x12, #56
	subs	x13, x13, #1
	b.ne	LBB3_14
	b	LBB3_11
LBB3_15:
	str	d0, [sp]
Lloh18:
	adrp	x0, l_.str@PAGE
Lloh19:
	add	x0, x0, l_.str@PAGEOFF
	bl	_printf
	mov	w0, #0                          ; =0x0
	ldp	x29, x30, [sp, #48]             ; 16-byte Folded Reload
	ldp	x20, x19, [sp, #32]             ; 16-byte Folded Reload
	ldp	x22, x21, [sp, #16]             ; 16-byte Folded Reload
	add	sp, sp, #64
	ret
	.loh AdrpAdd	Lloh12, Lloh13
	.loh AdrpAdd	Lloh10, Lloh11
	.loh AdrpAdd	Lloh14, Lloh15
	.loh AdrpAdd	Lloh16, Lloh17
	.loh AdrpAdd	Lloh18, Lloh19
	.cfi_endproc
                                        ; -- End function
	.section	__DATA,__data
	.globl	_bodies                         ; @bodies
	.p2align	4, 0x0
_bodies:
	.quad	0x0000000000000000              ; double 0
	.quad	0x0000000000000000              ; double 0
	.quad	0x0000000000000000              ; double 0
	.quad	0x0000000000000000              ; double 0
	.quad	0x0000000000000000              ; double 0
	.quad	0x0000000000000000              ; double 0
	.quad	0x4043bd3cc9be45de              ; double 39.478417604357432
	.quad	0x40135da0343cd92c              ; double 4.8414314424647209
	.quad	0xbff290abc01fdb7c              ; double -1.1603200440274284
	.quad	0xbfba86f96c25ebf0              ; double -0.10362204447112311
	.quad	0x3fe367069b93ccbc              ; double 0.60632639299583202
	.quad	0x40067ef2f57d949b              ; double 2.8119868449162602
	.quad	0xbf99d2d79a5a0715              ; double -0.025218361659887629
	.quad	0x3fa34c95d9ab33d8              ; double 0.037693674870389493
	.quad	0x4020afcdc332ca67              ; double 8.3433667182445799
	.quad	0x40107fcb31de01b0              ; double 4.1247985641243048
	.quad	0xbfd9d353e1eb467c              ; double -0.40352341711432138
	.quad	0xbff02c21b8879442              ; double -1.0107743461787924
	.quad	0x3ffd35e9bf1f8f13              ; double 1.8256623712304119
	.quad	0x3f813c485f1123b4              ; double 0.0084157613765841535
	.quad	0x3f871d490d07c637              ; double 0.011286326131968767
	.quad	0x4029c9eacea7d9cf              ; double 12.894369562139131
	.quad	0xc02e38e8d626667e              ; double -15.111151401698631
	.quad	0xbfcc9557be257da0              ; double -0.22330757889265573
	.quad	0x3ff1531ca9911bef              ; double 1.0827910064415354
	.quad	0x3febcc7f3e54bbc5              ; double 0.86871301816960822
	.quad	0xbf862f6bfaf23e7c              ; double -0.010832637401363636
	.quad	0x3f5c3dd29cf41eb3              ; double 0.0017237240570597112
	.quad	0x402ec267a905572a              ; double 15.379697114850917
	.quad	0xc039eb5833c8a220              ; double -25.919314609987964
	.quad	0x3fc6f1f393abe540              ; double 0.17925877295037118
	.quad	0x3fef54b61659bc4a              ; double 0.97909073224389798
	.quad	0x3fe307c631c4fba3              ; double 0.59469899864767617
	.quad	0xbfa1cb88587665f6              ; double -0.034755955504078104
	.quad	0x3f60a8f3531799ac              ; double 0.0020336868699246304

	.section	__TEXT,__cstring,cstring_literals
l_.str:                                 ; @.str
	.asciz	"%.9f\n"

.subsections_via_symbols

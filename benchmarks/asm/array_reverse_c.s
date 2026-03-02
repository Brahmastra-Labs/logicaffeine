	.section	__TEXT,__text,regular,pure_instructions
	.build_version macos, 15, 0	sdk_version 15, 2
	.globl	_main                           ; -- Begin function main
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
	subs	x8, x20, #1
	b.lt	LBB0_8
; %bb.3:
	mov	x9, #0                          ; =0x0
	mov	w10, #42                        ; =0x2a
	mov	w11, #20077                     ; =0x4e6d
	movk	w11, #16838, lsl #16
	mov	w12, #12345                     ; =0x3039
LBB0_4:                                 ; =>This Inner Loop Header: Depth=1
	madd	x13, x10, x11, x12
	and	x10, x13, #0x7fffffff
	ubfx	x13, x13, #16, #15
	str	x13, [x19, x9, lsl #3]
	add	x9, x9, #1
	cmp	x20, x9
	b.ne	LBB0_4
; %bb.5:
	cmp	x20, #2
	b.lt	LBB0_8
; %bb.6:
	mov	x9, #0                          ; =0x0
	mov	x10, x8
LBB0_7:                                 ; =>This Inner Loop Header: Depth=1
	lsl	x11, x9, #3
	ldr	x12, [x19, x11]
	lsl	x13, x10, #3
	ldr	x14, [x19, x13]
	str	x14, [x19, x11]
	str	x12, [x19, x13]
	add	x9, x9, #1
	sub	x10, x10, #1
	cmp	x9, x10
	b.lt	LBB0_7
LBB0_8:
	ldr	x9, [x19]
	ldr	x8, [x19, x8, lsl #3]
	cmp	x20, #0
	cinc	x10, x20, lt
	lsl	x10, x10, #2
	and	x10, x10, #0xfffffffffffffff8
	ldr	x10, [x19, x10]
	stp	x8, x10, [sp, #8]
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

# The shim's entry and its trap path. The boot ROM jumps to _shim_entry in machine mode with the inflated boot image's
# body address in a0. Every trap from the compiler (user mode) arrives at trap_entry, which saves the user's registers
# on the shim's own stack (mscratch holds its top while user code runs), calls trap() with the saved frame, and
# returns with whatever trap() left in it.

    .section .text.entry, "ax"
    .globl _shim_entry
_shim_entry:
    mv      s0, a0
    la      t0, __bss_start
    la      t1, __bss_end
1:  bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:  mv      a0, s0
    la      sp, shim_stack_top
    csrw    mscratch, sp
    la      t0, trap_entry
    csrw    mtvec, t0
    call    shim_main
1:  wfi
    j       1b

    .text
    .align  2
    .globl  trap_entry
trap_entry:
    csrrw   sp, mscratch, sp
    addi    sp, sp, -256
    sd      x1, 8(sp)
    sd      x3, 24(sp)
    sd      x4, 32(sp)
    sd      x5, 40(sp)
    sd      x6, 48(sp)
    sd      x7, 56(sp)
    sd      x8, 64(sp)
    sd      x9, 72(sp)
    sd      x10, 80(sp)
    sd      x11, 88(sp)
    sd      x12, 96(sp)
    sd      x13, 104(sp)
    sd      x14, 112(sp)
    sd      x15, 120(sp)
    sd      x16, 128(sp)
    sd      x17, 136(sp)
    sd      x18, 144(sp)
    sd      x19, 152(sp)
    sd      x20, 160(sp)
    sd      x21, 168(sp)
    sd      x22, 176(sp)
    sd      x23, 184(sp)
    sd      x24, 192(sp)
    sd      x25, 200(sp)
    sd      x26, 208(sp)
    sd      x27, 216(sp)
    sd      x28, 224(sp)
    sd      x29, 232(sp)
    sd      x30, 240(sp)
    sd      x31, 248(sp)
    csrr    t0, mscratch
    sd      t0, 16(sp)
    sd      zero, 0(sp)
    addi    t0, sp, 256
    csrw    mscratch, t0
    mv      a0, sp
    call    trap
    ld      x1, 8(sp)
    ld      x3, 24(sp)
    ld      x4, 32(sp)
    ld      x5, 40(sp)
    ld      x6, 48(sp)
    ld      x7, 56(sp)
    ld      x8, 64(sp)
    ld      x9, 72(sp)
    ld      x10, 80(sp)
    ld      x11, 88(sp)
    ld      x12, 96(sp)
    ld      x13, 104(sp)
    ld      x14, 112(sp)
    ld      x15, 120(sp)
    ld      x16, 128(sp)
    ld      x17, 136(sp)
    ld      x18, 144(sp)
    ld      x19, 152(sp)
    ld      x20, 160(sp)
    ld      x21, 168(sp)
    ld      x22, 176(sp)
    ld      x23, 184(sp)
    ld      x24, 192(sp)
    ld      x25, 200(sp)
    ld      x26, 208(sp)
    ld      x27, 216(sp)
    ld      x28, 224(sp)
    ld      x29, 232(sp)
    ld      x30, 240(sp)
    ld      x31, 248(sp)
    ld      x2, 16(sp)
    mret

# enter_user(entry, sp): start the compiler in user mode at `entry` with its stack at `sp` and every other
# register zero, as Linux starts a process.
    .globl  enter_user
enter_user:
    csrw    mepc, a0
    li      t0, (3 << 11) | (1 << 7)
    csrc    mstatus, t0
    mv      sp, a1
    li      x1, 0
    li      x3, 0
    li      x4, 0
    li      x5, 0
    li      x6, 0
    li      x7, 0
    li      x8, 0
    li      x9, 0
    li      x10, 0
    li      x11, 0
    li      x12, 0
    li      x13, 0
    li      x14, 0
    li      x15, 0
    li      x16, 0
    li      x17, 0
    li      x18, 0
    li      x19, 0
    li      x20, 0
    li      x21, 0
    li      x22, 0
    li      x23, 0
    li      x24, 0
    li      x25, 0
    li      x26, 0
    li      x27, 0
    li      x28, 0
    li      x29, 0
    li      x30, 0
    li      x31, 0
    mret

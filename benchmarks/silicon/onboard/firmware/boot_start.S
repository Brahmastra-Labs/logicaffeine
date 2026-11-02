# The hart's reset vector. The boot ROM runs from block RAM with its stack at the top of the scratch RAM, its zeroed
# data below; it holds no initialized writable data, so nothing needs copying out of the ROM.
    .section .text.entry, "ax"
    .globl _start
_start:
    la      t0, __bss_start
    la      t1, __bss_end
1:  bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:  la      sp, boot_stack_top
    call    boot_main
3:  wfi
    j       3b

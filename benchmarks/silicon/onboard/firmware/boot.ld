/* The boot ROM: code and constants in the 16 KB ROM, zeroed data and the stack in the 8 KB scratch RAM. */
OUTPUT_ARCH(riscv)
ENTRY(_start)
SECTIONS
{
  /* The reset vector is the ROM's first byte: no note or other loaded section may come before the code. */
  /DISCARD/ : { *(.note .note.*) *(.comment) }
  . = 0x00010000;
  .text : { *(.text.entry) *(.text .text.*) }
  ASSERT(_start == 0x00010000, "the boot ROM's first instruction is not _start")
  .rodata : { *(.rodata .rodata.* .srodata .srodata.*) }
  ASSERT(. <= 0x00010000 + 0x4000, "the boot ROM does not fit its 16 KB")
  .data : { *(.data .data.* .sdata .sdata.*) }
  ASSERT(SIZEOF(.data) == 0, "the boot ROM may hold no initialized writable data")
  . = 0x00020000;
  __bss_start = .;
  .bss : { *(.bss .bss.* .sbss .sbss.* COMMON) }
  . = ALIGN(8);
  __bss_end = .;
  boot_stack_top = 0x00020000 + 0x2000 - 0x10;
  ASSERT(__bss_end <= 0x00020000 + 0x1000, "the boot ROM's data leaves too little stack")
}

/* The shim in the first 2 MB of the DDR2: its code and data from the bottom, its stack down from the top. */
OUTPUT_ARCH(riscv)
ENTRY(_shim_entry)
SECTIONS
{
  /* The boot ROM jumps to the shim's first byte: no note or other loaded section may come before the code. */
  /DISCARD/ : { *(.note .note.*) *(.comment) }
  . = 0x80000000;
  .text : { *(.text.entry) *(.text .text.*) }
  ASSERT(_shim_entry == 0x80000000, "the shim's first instruction is not _shim_entry")
  .rodata : { *(.rodata .rodata.* .srodata .srodata.*) }
  .data : { *(.data .data.* .sdata .sdata.*) }
  . = ALIGN(8);
  __bss_start = .;
  .bss : { *(.bss .bss.* .sbss .sbss.* COMMON) }
  . = ALIGN(8);
  __bss_end = .;
  shim_stack_top = 0x80000000 + 0x200000 - 0x40;
  ASSERT(__bss_end <= 0x80000000 + 0x200000 - 0x10000, "the shim and its stack overlap")
}

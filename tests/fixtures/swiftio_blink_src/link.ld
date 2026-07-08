ENTRY(Reset_Handler)
MEMORY { SDRAM (rwx) : ORIGIN = 0x80000000, LENGTH = 32M }
SECTIONS {
  .vectors 0x80000000 : { KEEP(*(.vectors)) } > SDRAM
  .text   : { *(.text*) }   > SDRAM
  .rodata : { *(.rodata*) } > SDRAM
  .data   : { *(.data*) }   > SDRAM
  .bss    : { *(.bss*) *(COMMON) } > SDRAM
}

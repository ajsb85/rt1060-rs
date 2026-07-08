/* Minimal Teensy 4.1 flash image: FCFB + IVT + boot_data + first-stage + the
   OCRAM-linked SwiftIO payload. Mirrors teensy_cores/teensy4/imxrt1062_t41.ld
   headers but only what the first-stage needs. */
MEMORY
{
    FLASH (rx): ORIGIN = 0x60000000, LENGTH = 7936K
    DTCM  (rw): ORIGIN = 0x20000000, LENGTH = 512K
}
ENTRY(ImageVectorTable)
SECTIONS
{
    .text.headers : {
        KEEP(*(.flashconfig))
        FILL(0xFF)
        . = ORIGIN(FLASH) + 0x1000;
        KEEP(*(.ivt))
        KEEP(*(.bootdata))
        . = ALIGN(1024);
    } > FLASH

    .text.code : {
        KEEP(*(.startup))
        *(.text*)
        *(.rodata*)
        . = ALIGN(4);
    } > FLASH

    .csf : {
        KEEP(*(.csf))
        . = ALIGN(4);
    } > FLASH

    .payload : {
        . = ALIGN(4);
        KEEP(*(.payload))
        . = ALIGN(1024);
    } > FLASH

    __text_csf_end = .;
    _flashimagelen = __text_csf_end - ORIGIN(FLASH);
    _estack = ORIGIN(DTCM) + LENGTH(DTCM);   /* transient first-stage stack */

    /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) *(.comment) *(.noinit*) }
}

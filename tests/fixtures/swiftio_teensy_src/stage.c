// SPDX-License-Identifier: MIT
// Minimal i.MX RT1062 first-stage ("eboot") for booting a RAM-linked image on
// a Teensy 4.1: the Boot ROM validates the FlexSPI config at 0x60000000, reads
// the IVT at 0x60001000, and branches to ResetHandler. We copy the payload
// (a SwiftIO/Zephyr image linked to OCRAM 0x20200000, appended in flash) into
// OCRAM, point VTOR at it, and jump to its reset vector. We deliberately do NOT
// touch FlexRAM/clocks/fast-GPIO — the SwiftIO stack configures the SoC itself,
// and enabling the GPIO6-9 fast aliases here would shadow SwiftIO's GPIO2 pin.
#include <stdint.h>

extern uint32_t _payload_start;
extern uint32_t _payload_end;

#define OCRAM_BASE 0x20200000u
#define SCB_VTOR (*(volatile uint32_t *)0xE000ED08u)

__attribute__((used)) void stage_main(void) {
    volatile uint32_t *src = &_payload_start;
    volatile uint32_t *end = &_payload_end;
    volatile uint32_t *dst = (volatile uint32_t *)OCRAM_BASE;
    while (src < end) *dst++ = *src++;
    SCB_VTOR = OCRAM_BASE;
    uint32_t sp = ((volatile uint32_t *)OCRAM_BASE)[0];
    uint32_t pc = ((volatile uint32_t *)OCRAM_BASE)[1];
    __asm__ volatile("msr msp, %0\n\tmsr psp, %0\n\tisb\n\tbx %1"
                     :: "r"(sp), "r"(pc) : "memory");
    __builtin_unreachable();
}

// The ROM enters here (IVT.entry). Set a private stack (top of DTCM, which the
// copy below never touches) before running any C, then hand off.
__attribute__((section(".startup"), naked, used)) void ResetHandler(void) {
    __asm__ volatile("ldr sp, =_estack\n\tbl stage_main");
}

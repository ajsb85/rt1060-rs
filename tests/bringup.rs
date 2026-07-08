// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Boot bring-up: replay, at the bus level, the exact status/handshake
//! spin-loops that the NXP SDK `BOARD_BootClockRUN` + `SEMC_ConfigureSDRAM`
//! run, and assert each poll bit reads the value that lets firmware proceed
//! (so a real image will not hang). Then drive the SwiftIO Micro RGB LED the
//! way the Zephyr igpio driver does and observe it.
//!
//! Register offsets/bits are cited from `fsl_clock.c` / `fsl_semc.c` /
//! `clock_config.c` and `MIMXRT1062.h`.

use rt1060_rs::Rt1060;

const CCM_ANALOG: u32 = 0x400D_8000;
const CCM: u32 = 0x400F_C000;
const XTALOSC24M: u32 = 0x400D_8000;
const SEMC: u32 = 0x402F_0000;
const DCDC: u32 = 0x4008_0000;
const GPIO1: u32 = 0x401B_8000;
const IOMUXC: u32 = 0x401F_8000;

#[test]
fn clock_and_sdram_bringup_polls_all_terminate() {
    let mut soc = Rt1060::new();
    soc.quiet();
    let bus = &mut soc.bus;

    // --- CLOCK_InitExternalClk: XTAL power-up + 24 MHz OK ---
    // XTALOSC24M.LOWPWR_CTRL bit 16 (XTALOSC_PWRUP_STAT); MISC0 bit 15
    // (OSC_XTALOK). fsl_clock.c CLOCK_InitExternalClk.
    assert_ne!(
        bus.read32(XTALOSC24M + 0x270) & (1 << 16),
        0,
        "XTAL powered"
    );
    assert_ne!(bus.read32(CCM_ANALOG + 0x150) & (1 << 15), 0, "24M OK");

    // --- DCDC VDD_SOC ramp: REG0.STS_DC_OK bit 31 (clock_config.c) ---
    bus.write32(DCDC + 0x0C, 0x13); // REG3.TRG = 1.275 V
    assert_ne!(bus.read32(DCDC) & (1 << 31), 0, "DCDC DC_OK");

    // --- CLOCK_InitArmPll: bypass, DIV_SELECT=100|ENABLE, poll LOCK ---
    // PLL_ARM at CCM_ANALOG+0x00; LOCK is bit 31 (fsl_clock.c:591).
    bus.write32(CCM_ANALOG + 0x04, 1 << 16); // PLL_ARM_SET: BYPASS
    bus.write32(CCM_ANALOG, 100 | (1 << 13)); // DIV_SELECT=100, ENABLE
    assert_ne!(bus.read32(CCM_ANALOG) & (1 << 31), 0, "ARM PLL LOCK");
    bus.write32(CCM_ANALOG + 0x08, 1 << 16); // PLL_ARM_CLR: BYPASS
    // SYS PLL (0x30) and USB1 PLL (0x10) LOCK poll likewise.
    assert_ne!(bus.read32(CCM_ANALOG + 0x30) & (1 << 31), 0, "SYS PLL LOCK");
    assert_ne!(
        bus.read32(CCM_ANALOG + 0x10) & (1 << 31),
        0,
        "USB1 PLL LOCK"
    );

    // --- CLOCK_SetDiv(ArmDiv,1) / SetMux: CCM.CDHIPR handshake not busy ---
    // fsl_clock.h: while (CCM->CDHIPR & (1<<busyShift)) {}. CDHIPR at 0x48.
    assert_eq!(bus.read32(CCM + 0x48), 0, "no clock handshake ever busy");

    // --- SEMC_Init: MCR.SWRST (bit 0) self-clears ---
    bus.write32(SEMC, 0x1); // MCR.SWRST
    assert_eq!(bus.read32(SEMC) & 0x1, 0, "SWRST self-cleared");

    // --- SEMC_ConfigureSDRAM: each IP command sets INTR.IPCMDDONE (bit 0) ---
    // fsl_semc.c SEMC_SendIPCommand writes IPCMD (0xB0) with KEY 0xA55A.
    bus.write32(SEMC + 0xB0, (0xA55A << 16) | 0x8); // keyed MODESET
    let intr = bus.read32(SEMC + 0x14);
    assert_ne!(intr & 0x1, 0, "IPCMDDONE set");
    assert_eq!(intr & 0x2, 0, "IPCMDERR clear");
}

#[test]
fn swiftio_600mhz_clock_config_reads_back() {
    let mut soc = Rt1060::new();
    soc.quiet();
    // BOARD_BootClockRUN: ARM PLL DIV_SELECT=100, ARM_PODF=1 (÷2), AHB_PODF=0,
    // IPG_PODF=3 (÷4), PERCLK from OSC 24 MHz.
    soc.bus.write32(CCM_ANALOG, 100 | (1 << 13)); // PLL_ARM: DIV_SELECT=100|ENABLE
    soc.bus.write32(CCM + 0x10, 1); // CACRR.ARM_PODF = 1
    soc.bus.write32(CCM + 0x14, 3 << 8); // CBCDR: IPG_PODF=3
    soc.bus.write32(CCM + 0x18, 3 << 18); // CBCMR: PRE_PERIPH_CLK_SEL = PLL1
    soc.bus.write32(CCM + 0x1C, 1 << 6); // CSCMR1: PERCLK from OSC 24 MHz
    assert_eq!(soc.core_hz(), 600_000_000, "core = 600 MHz");
    assert_eq!(soc.perclk_hz(), 24_000_000, "perclk = 24 MHz");
    assert_eq!(soc.clocks().ipg, 150_000_000, "ipg = 150 MHz");
}

#[test]
fn gpt_counts_in_the_perclk_domain() {
    const GPT1: u32 = 0x401E_C000;
    let mut soc = Rt1060::new();
    soc.quiet();
    // 600 MHz core, 24 MHz PERCLK (25:1 ratio).
    soc.bus.write32(CCM_ANALOG, 100 | (1 << 13));
    soc.bus.write32(CCM + 0x10, 1); // ARM_PODF
    soc.bus.write32(CCM + 0x18, 3 << 18); // PRE_PERIPH = PLL1
    soc.bus.write32(CCM + 0x1C, 1 << 6); // PERCLK from OSC 24 MHz
    assert_eq!(soc.core_hz(), 600_000_000);
    assert_eq!(soc.perclk_hz(), 24_000_000);
    // GPT1: CR.CLKSRC = 1 (PERCLK), EN. PR = 0 (÷1).
    soc.bus.write32(GPT1, (1 << 6) | 1);
    // 600 core cycles → 600 * 24/600 = 24 PERCLK ticks → CNT = 24.
    soc.bus.periph.tick(600);
    assert_eq!(
        soc.bus.read32(GPT1 + 0x24),
        24,
        "GPT ran at PERCLK, not core"
    );
    // A GPT with CLKSRC = 0 (stopped) does not count.
    const GPT2: u32 = 0x401F_0000;
    soc.bus.write32(GPT2, 1); // EN but CLKSRC = 0
    soc.bus.periph.tick(600);
    assert_eq!(soc.bus.read32(GPT2 + 0x24), 0, "no clock source = stopped");
}

#[test]
fn edma_moves_a_block_through_the_bus() {
    const DMA: u32 = 0x400E_8000;
    let mut soc = Rt1060::new();
    soc.quiet();
    // Eight source words in SDRAM.
    let (src, dst) = (0x8000_0000, 0x8000_1000);
    for i in 0..8u32 {
        soc.bus.write32(src + i * 4, 0x1000 + i);
    }
    // Program TCD channel 0 through the bus (32-bit and packed-16-bit fields).
    let tcd = DMA + 0x1000;
    soc.bus.write32(tcd, src); // SADDR (offset 0x00)
    soc.bus.write16(tcd + 0x04, 4); // SOFF = +4
    soc.bus.write16(tcd + 0x06, (2 << 8) | 2); // ATTR: SSIZE=DSIZE=32-bit
    soc.bus.write32(tcd + 0x08, 32); // NBYTES = 32 (8 words)
    soc.bus.write32(tcd + 0x10, dst); // DADDR
    soc.bus.write16(tcd + 0x14, 4); // DOFF = +4
    soc.bus.write16(tcd + 0x16, 1); // CITER = 1
    soc.bus.write16(tcd + 0x1E, 1); // BITER = 1
    soc.bus.write16(tcd + 0x1C, 1 << 1); // CSR: INTMAJOR
    // Software start via the SSRT action register (byte at 0x1D) → the bus
    // services the engine at strobe time.
    soc.bus.write8(DMA + 0x1D, 0);
    for i in 0..8u32 {
        assert_eq!(soc.bus.read32(dst + i * 4), 0x1000 + i, "word {i} DMA'd");
    }
    // Major-complete interrupt latched for channel 0 → DMA IRQ 0.
    assert_ne!(soc.bus.read32(DMA + 0x24) & 1, 0, "INT[0] pending");
    use rt1060_rs::cortex_m::Bus;
    assert!(soc.bus.irq_lines().test(0), "DMA0 = IRQ 0");
}

#[test]
fn rgb_led_red_turns_on_active_low() {
    let mut soc = Rt1060::new();
    soc.quiet();

    // IOMUXC: mux pad GPIO_AD_B0_09 to ALT5 (GPIO1_IO09). SW_MUX_CTL_PAD is
    // an array at IOMUXC+0x14; the AD_B0_09 entry is at absolute 0x401F80E0
    // (fsl_iomuxc.h). Writing 0x5 selects GPIO. (Observation works via GPIO1
    // regardless; this exercises the pad-mux path staying stored-readback.)
    soc.bus.write32(0x401F_80E0, 0x5);
    assert_eq!(soc.bus.read32(0x401F_80E0), 0x5, "mux write sticks");
    let _ = IOMUXC; // base documented above

    // Zephyr gpio_mcux_igpio: GDIR |= 1<<9 (output), then DR_CLEAR = 1<<9
    // to drive low. Active-low RGB → red LED ON.
    soc.bus.write32(GPIO1 + 0x04, 1 << 9); // GDIR: pin 9 output
    soc.bus.write32(GPIO1 + 0x88, 1 << 9); // DR_CLEAR: drive low
    assert_eq!(soc.led_rgb(), (true, false, false), "red on, others off");

    // DR_SET drives high → red LED OFF.
    soc.bus.write32(GPIO1 + 0x84, 1 << 9); // DR_SET
    assert_eq!(soc.led_rgb(), (false, false, false), "red off");

    // Blue on: GPIO1 pin 11 output, driven low.
    soc.bus.write32(GPIO1 + 0x04, (1 << 9) | (1 << 11));
    soc.bus.write32(GPIO1 + 0x88, 1 << 11);
    assert_eq!(soc.led_rgb(), (false, false, true), "blue on");
}

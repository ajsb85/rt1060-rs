// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Clock-tree frequency computation (MIMXRT1062, RM §14; `fsl_clock.c`
//! `CLOCK_GetFreq`).
//!
//! Derives the clock roots firmware cares about from the CCM + CCM_ANALOG
//! registers it programmed: the CPU/AHB clock, IPG, PERCLK (the GPT/PIT time
//! base) and the LPUART serial clock. This turns the emulator's "1 instruction
//! = 1 core cycle" time base into a real ratio so timers tick at the right
//! rate relative to the core, and gives LPUART a real baud reference.
//!
//! The ARM-PLL → CPU path (the SwiftIO Micro 600 MHz configuration) is modeled
//! exactly; the PLL2/PFD and PLL3 branches use their nominal frequencies
//! (528 / 396 / 352 / 480 MHz) since the PFD fractional dividers are not yet
//! modeled (ROADMAP). Fields cited from MIMXRT1062.h.

use super::analog::CcmAnalog;
use super::ccm::Ccm;

/// 24 MHz crystal oscillator — the root of the whole tree.
pub const OSC24M: u64 = 24_000_000;
/// Nominal SYS PLL (PLL2) output.
pub const PLL_SYS: u64 = 528_000_000;
/// Nominal USB1 PLL (PLL3) output; the LPUART default root is PLL3/6.
pub const PLL_USB1: u64 = 480_000_000;

/// A snapshot of the clock roots, in Hz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clocks {
    pub osc: u64,
    /// ARM PLL (PLL1) VCO output.
    pub pll_arm: u64,
    /// CPU / AHB clock root (`kCLOCK_CpuClk`).
    pub core: u64,
    pub ipg: u64,
    /// PERCLK — the GPT/PIT counter clock.
    pub perclk: u64,
    /// LPUART serial clock root (before the peripheral's own divider).
    pub uart: u64,
}

impl Clocks {
    /// Compute the roots from the current CCM / CCM_ANALOG register state.
    pub fn compute(ccm: &Ccm, analog: &CcmAnalog) -> Self {
        let osc = OSC24M;

        // PLL_ARM: Fout = OSC * DIV_SELECT / 2 (CCM_ANALOG_PLL_ARM, 0x00;
        // DIV_SELECT = bits [6:0]).
        let div_select = u64::from(analog.reg(0x00) & 0x7F);
        let pll_arm = osc * div_select / 2;

        // CCM bus/mux registers.
        let cacrr = ccm.reg(0x10); // ARM_PODF [2:0]
        let cbcdr = ccm.reg(0x14); // IPG_PODF [9:8], AHB_PODF [12:10], PERIPH_CLK_SEL [25]
        let cbcmr = ccm.reg(0x18); // PRE_PERIPH_CLK_SEL [19:18]
        let cscmr1 = ccm.reg(0x1C); // PERCLK_PODF [5:0], PERCLK_CLK_SEL [6]
        let cscdr1 = ccm.reg(0x24); // UART_CLK_PODF [5:0], UART_CLK_SEL [6]

        let arm_podf = u64::from((cacrr & 0x7) + 1);
        let ahb_podf = u64::from(((cbcdr & 0x1C00) >> 10) + 1);
        let ipg_podf = u64::from(((cbcdr & 0x300) >> 8) + 1);
        let periph_clk_sel = (cbcdr >> 25) & 1;
        let pre_periph = (cbcmr >> 18) & 3;

        // PLL2 (SYS) PFD outputs = PLL_SYS × 18 / FRAC (CCM_ANALOG_PFD_528,
        // 0x100; PFD0_FRAC [5:0], PFD2_FRAC [21:16]).
        let pfd528 = analog.reg(0x100);
        let pfd = |frac: u32| -> u64 {
            let f = u64::from(frac & 0x3F);
            if f == 0 { 0 } else { PLL_SYS * 18 / f }
        };
        let pll2_pfd0 = pfd(pfd528);
        let pll2_pfd2 = pfd(pfd528 >> 16);

        // periph_clk = the pre-AHB source (fsl_clock.c CLOCK_GetPeriphClkFreq).
        let periph = if periph_clk_sel == 1 {
            // periph_clk2 path — its own mux defaults to the 24 MHz OSC.
            osc
        } else {
            match pre_periph {
                0 => PLL_SYS,                               // PLL2
                1 => pll2_pfd2,                             // PLL2_PFD2
                2 => pll2_pfd0,                             // PLL2_PFD0
                3 if div_select != 0 => pll_arm / arm_podf, // PLL1 / ARM_PODF
                _ => 0,
            }
        };
        // Reset / unconfigured guard: fall back to the OSC so we never report
        // a 0 Hz (or divide-by-nonsense) core before firmware runs clock init.
        let core = (periph / ahb_podf).max(osc);
        let ipg = (core / ipg_podf).max(1);

        // PERCLK: OSC (sel=1) or IPG (sel=0), then PERCLK_PODF.
        let perclk_podf = u64::from((cscmr1 & 0x3F) + 1);
        let perclk_root = if (cscmr1 >> 6) & 1 == 1 { osc } else { ipg };
        let perclk = (perclk_root / perclk_podf).max(1);

        // UART: PLL3/6 (sel=0) or OSC (sel=1), then UART_CLK_PODF.
        let uart_podf = u64::from((cscdr1 & 0x3F) + 1);
        let uart_root = if (cscdr1 >> 6) & 1 == 1 {
            osc
        } else {
            PLL_USB1 / 6
        };
        let uart = (uart_root / uart_podf).max(1);

        Clocks {
            osc,
            pll_arm,
            core,
            ipg,
            perclk,
            uart,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_defaults_are_sane() {
        // Fresh CCM/analog: never 0 Hz.
        let c = Clocks::compute(&Ccm::new(), &CcmAnalog::new());
        assert!(c.core >= OSC24M);
        assert!(c.perclk >= 1);
        assert!(c.uart >= 1);
    }

    #[test]
    fn swiftio_600mhz_path() {
        // Reproduce BOARD_BootClockRUN: ARM PLL DIV_SELECT=100 (VCO 1200 MHz),
        // ARM_PODF=1 (÷2 → 600 MHz periph), AHB_PODF=0 (÷1), IPG_PODF=3 (÷4 →
        // 150 MHz), PERCLK from OSC 24 MHz ÷1.
        let mut ccm = Ccm::new();
        let mut analog = CcmAnalog::new();
        analog.write(0x00, 100); // PLL_ARM.DIV_SELECT = 100
        ccm.write(0x10, 1); // CACRR.ARM_PODF = 1 (divide by 2)
        // CBCDR: AHB_PODF=0, IPG_PODF=3 (<<8), PERIPH_CLK_SEL=0.
        ccm.write(0x14, 3 << 8);
        ccm.write(0x18, 3 << 18); // CBCMR.PRE_PERIPH_CLK_SEL = 3 (PLL1)
        ccm.write(0x1C, 1 << 6); // CSCMR1.PERCLK_CLK_SEL = 1 (OSC), PODF=0
        let c = Clocks::compute(&ccm, &analog);
        assert_eq!(c.pll_arm, 1_200_000_000);
        assert_eq!(c.core, 600_000_000);
        assert_eq!(c.ipg, 150_000_000);
        assert_eq!(c.perclk, 24_000_000);
    }

    #[test]
    fn pll2_pfd_derived_core_clock() {
        // PRE_PERIPH = 2 selects PLL2_PFD0. Default FRAC0 = 27 → 528*18/27 =
        // 352 MHz; reprogram FRAC0 = 18 → 528*18/18 = 528 MHz.
        let mut ccm = Ccm::new();
        let analog = CcmAnalog::new();
        ccm.write(0x18, 2 << 18); // CBCMR.PRE_PERIPH_CLK_SEL = PLL2_PFD0
        assert_eq!(Clocks::compute(&ccm, &analog).core, 352_000_000);

        let mut analog = CcmAnalog::new();
        // PFD_528 FRAC0 = 18 (keep the other lanes' defaults).
        analog.write(0x100, 18 | (16 << 8) | (24 << 16) | (16 << 24));
        assert_eq!(Clocks::compute(&ccm, &analog).core, 528_000_000);
    }

    #[test]
    fn uart_root_pll3_div6_default() {
        let c = Clocks::compute(&Ccm::new(), &CcmAnalog::new());
        // UART_CLK_SEL=0 → PLL3/6 = 80 MHz, PODF=0 → 80 MHz.
        assert_eq!(c.uart, 80_000_000);
    }
}

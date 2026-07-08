// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! CCM_ANALOG / PMU / XTALOSC24M — the analog clock/power window at
//! `0x400D_8000` (MIMXRT1062.h; the three logical blocks overlap this one
//! 16 KiB window).
//!
//! Two things boot-time clock init depends on:
//!
//! 1. **The i.MX SET/CLR/TOG register-quad convention.** Every register `R`
//!    has write-only aliases `R+4` (SET, `reg |= v`), `R+8` (CLR,
//!    `reg &= !v`), `R+C` (TOG, `reg ^= v`); reads through any alias return
//!    the base value. The whole analog window is laid out on 0x10 strides,
//!    so `base = off & !0xC`, `op = (off >> 2) & 3`. (`fsl_clock.c` uses
//!    `MISC0_SET`/`MISC0_CLR`, `PLL_ARM_SET`/`_CLR`, … extensively.)
//! 2. **Status bits firmware spins on** must read back asserted (or the
//!    boot hangs). We force them on read regardless of the stored value:
//!    * every PLL LOCK = bit 31 (`CCM_ANALOG_PLL_*_LOCK_MASK`,
//!      `fsl_clock.c` `CLOCK_Init*Pll` poll loops);
//!    * `MISC0` (0x150) bit 15 `OSC_XTALOK` (`CLOCK_InitExternalClk`);
//!    * `LOWPWR_CTRL` (0x270) bit 16 `XTALOSC_PWRUP_STAT`.
//!
//! PLL frequency math (dividers → real clock roots) is a later ROADMAP item;
//! here we only need the spin-loops to terminate. For reference, the SwiftIO
//! Micro 600 MHz path writes `PLL_ARM.DIV_SELECT = 100` → VCO = 24 MHz*100/2
//! = 1200 MHz, then `ARM_PODF = 2` → 600 MHz (`clock_config.c`).

/// PLL register offsets carrying a LOCK bit at 31 (MIMXRT1062.h CCM_ANALOG).
const PLL_ARM: u32 = 0x00;
const PLL_USB1: u32 = 0x10;
const PLL_USB2: u32 = 0x20;
const PLL_SYS: u32 = 0x30;
const PLL_AUDIO: u32 = 0x70;
const PLL_VIDEO: u32 = 0xA0;
const PLL_ENET: u32 = 0xE0;
const MISC0: u32 = 0x150;
const LOWPWR_CTRL: u32 = 0x270;

pub struct CcmAnalog {
    regs: Box<[u32; 4096]>,
}

impl CcmAnalog {
    pub fn new() -> Self {
        let mut regs = Box::new([0u32; 4096]);
        // PFD reset FRAC defaults so the clock tree computes nominal PFD
        // outputs before firmware reprograms them. Each byte: [5:0] = FRAC.
        // PFD_528 (0x100): FRAC0=27 → 528*18/27 = 352 MHz (PLL2_PFD0),
        //                  FRAC2=24 → 528*18/24 = 396 MHz (PLL2_PFD2).
        regs[(0x100 >> 2) as usize] = 27 | (16 << 8) | (24 << 16) | (16 << 24);
        // PFD_480 (0x0F0): PLL3 PFDs (nominal ITU defaults).
        regs[(0x0F0 >> 2) as usize] = 12 | (16 << 8) | (17 << 16) | (19 << 24);
        CcmAnalog { regs }
    }

    #[inline]
    fn is_pll(base: u32) -> bool {
        matches!(
            base,
            PLL_ARM | PLL_USB1 | PLL_USB2 | PLL_SYS | PLL_AUDIO | PLL_VIDEO | PLL_ENET
        )
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        // Reads through SET/CLR/TOG aliases return the base register value.
        let base = offset & !0xC;
        let mut v = self.regs[(base >> 2) as usize & 4095];
        if Self::is_pll(base) {
            v |= 1 << 31; // *_LOCK
        } else if base == MISC0 {
            v |= 1 << 15; // OSC_XTALOK
        } else if base == LOWPWR_CTRL {
            v |= 1 << 16; // XTALOSC_PWRUP_STAT
        }
        v
    }

    /// Side-effect-free base-register snapshot (for the clock-tree
    /// computation): the stored value, without the forced status bits.
    #[inline]
    pub fn reg(&self, offset: u32) -> u32 {
        self.regs[((offset & !0xC) >> 2) as usize & 4095]
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        let base = offset & !0xC;
        let slot = &mut self.regs[(base >> 2) as usize & 4095];
        *slot = match (offset >> 2) & 3 {
            1 => *slot | value,  // SET
            2 => *slot & !value, // CLR
            3 => *slot ^ value,  // TOG
            _ => value,          // base register
        };
    }
}

impl Default for CcmAnalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pll_lock_forced_regardless_of_writes() {
        let mut a = CcmAnalog::new();
        // CLOCK_InitArmPll: BYPASS, then DIV_SELECT|ENABLE, then poll LOCK.
        a.write(PLL_ARM, 0x1_0000); // set BYPASS via base write
        a.write(PLL_ARM, 100 | 0x2000); // DIV_SELECT=100, ENABLE
        assert_ne!(a.read(PLL_ARM) & (1 << 31), 0, "ARM PLL LOCK reads set");
        assert_ne!(a.read(PLL_SYS) & (1 << 31), 0);
        assert_ne!(a.read(PLL_USB1) & (1 << 31), 0);
    }

    #[test]
    fn set_clr_tog_aliases() {
        let mut a = CcmAnalog::new();
        // MISC0_SET / _CLR / _TOG mutate the base register.
        a.write(MISC0 + 4, 0x0000_0044); // SET
        assert_eq!(a.read(MISC0) & 0xFF, 0x44, "SET ORs (LOCK bit is 15)");
        a.write(MISC0 + 8, 0x0000_0004); // CLR bit 2
        assert_eq!(a.read(MISC0) & 0xFF, 0x40);
        a.write(MISC0 + 0xC, 0x0000_0080); // TOG bit 7
        assert_ne!(a.read(MISC0) & 0x80, 0);
        // OSC_XTALOK (bit 15) is always forced high.
        assert_ne!(a.read(MISC0) & (1 << 15), 0);
    }

    #[test]
    fn xtalosc_pwrup_stat_forced() {
        let mut a = CcmAnalog::new();
        assert_ne!(a.read(LOWPWR_CTRL) & (1 << 16), 0);
    }
}

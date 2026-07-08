// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Peripheral register blocks for the i.MX RT1060 (MIMXRT1062).
//!
//! Unlike the Series-2 EFR32 (SET/CLR/TGL alias pages, Secure/Non-Secure
//! mirrors), the i.MX RT's peripherals are plain word-register blocks on the
//! AIPS buses. The bus routes a 32-bit access to the owning block by its
//! 16 KiB-aligned base (`addr & !0x3FFF`); peripherals see `offset < 0x4000`.
//!
//! Peripheral convention (keep uniform — the bus routes on it):
//! ```ignore
//! pub fn read(&mut self, offset: u32) -> u32;       // may have read side effects
//! pub fn write(&mut self, offset: u32, value: u32);
//! pub fn irq_pending(&self) -> bool;                 // where relevant
//! ```
//! Modeled blocks get real types; blocks the boot path only configures are
//! stored-readback [`RawRegs`]; anything unmapped lands in [`Peripherals::
//! unknown`] with a warn-once log (log-first strategy: the warning is the
//! TODO list for the next milestone). Register truth = the CMSIS header
//! `../legacy-mcux-sdk/devices/MIMXRT1062/MIMXRT1062.h` and the SVD
//! `../mcux-soc-svd/MIMXRT1062/MIMXRT1062.xml`; cite offsets in comments.

pub mod ccm;
pub mod gpio;
pub mod gpt;
pub mod lpuart;
pub mod src;
pub mod wdog;

use crate::cortex_m::IrqMask;

/// Readback-what-you-wrote register file for blocks where "the SDK reads
/// back its own config" suffices (IOMUXC, CCM_ANALOG, GPC, SNVS, …) — the
/// rp2350-rs / mg24-rs RawRegs pattern. One 16 KiB window (4096 words).
pub struct RawRegs {
    regs: Box<[u32; 4096]>,
    pub name: &'static str,
}

impl RawRegs {
    pub fn new(name: &'static str) -> Self {
        Self {
            regs: Box::new([0; 4096]),
            name,
        }
    }

    /// Pre-seed a register (reset values that firmware polls).
    pub fn with(mut self, offset: u32, value: u32) -> Self {
        self.regs[(offset >> 2) as usize & 4095] = value;
        self
    }

    #[inline]
    pub fn read(&self, offset: u32) -> u32 {
        self.regs[(offset >> 2) as usize & 4095]
    }

    #[inline]
    pub fn write(&mut self, offset: u32, value: u32) {
        self.regs[(offset >> 2) as usize & 4095] = value;
    }
}

// ---------------------------------------------------------------------------
// Peripheral base addresses (MIMXRT1062.h `*_BASE` macros).
// ---------------------------------------------------------------------------

/// 16 KiB-aligned peripheral base addresses (MIMXRT1062.h). The bus routes
/// on `addr & !0x3FFF`, so every entry here is 16 KiB-aligned.
pub mod base {
    // AIPS-1 .. AIPS-4 modeled blocks (subset; extend per ROADMAP).
    pub const DCDC: u32 = 0x4008_0000;
    pub const PIT: u32 = 0x4008_4000;
    pub const IOMUXC_GPR: u32 = 0x400A_C000;
    pub const WDOG1: u32 = 0x400B_8000;
    pub const RTWDOG: u32 = 0x400B_C000;
    pub const GPIO5: u32 = 0x400C_0000;
    pub const WDOG2: u32 = 0x400D_0000;
    pub const SNVS: u32 = 0x400D_4000;
    /// CCM_ANALOG / PMU / XTALOSC24M all share this analog window.
    pub const CCM_ANALOG: u32 = 0x400D_8000;
    pub const GPC: u32 = 0x400F_4000;
    pub const SRC: u32 = 0x400F_8000;
    pub const CCM: u32 = 0x400F_C000;
    pub const LPUART1: u32 = 0x4018_4000;
    pub const LPUART2: u32 = 0x4018_8000;
    pub const LPUART3: u32 = 0x4018_C000;
    pub const LPUART4: u32 = 0x4019_0000;
    pub const LPUART5: u32 = 0x4019_4000;
    pub const LPUART6: u32 = 0x4019_8000;
    pub const LPUART7: u32 = 0x4019_C000;
    pub const LPUART8: u32 = 0x401A_0000;
    pub const GPIO1: u32 = 0x401B_8000;
    pub const GPIO2: u32 = 0x401B_C000;
    pub const GPIO3: u32 = 0x401C_0000;
    pub const GPIO4: u32 = 0x401C_4000;
    pub const OCOTP: u32 = 0x401F_4000;
    pub const IOMUXC: u32 = 0x401F_8000;
    pub const GPT1: u32 = 0x401E_C000;
    pub const GPT2: u32 = 0x401F_0000;
    // High-speed GPIO (GPIO6..9) live on a separate 0x4200_0000 island.
    pub const GPIO6: u32 = 0x4200_0000;
    pub const GPIO7: u32 = 0x4200_4000;
    pub const GPIO8: u32 = 0x4200_8000;
    pub const GPIO9: u32 = 0x4200_C000;
}

// ---------------------------------------------------------------------------
// IRQ numbers (MIMXRT1062.h IRQn_Type; external, 0-based).
// ---------------------------------------------------------------------------

/// External interrupt numbers used by the modeled peripherals.
pub mod irq {
    pub const LPUART1: u32 = 20;
    pub const GPIO1_COMBINED_0_15: u32 = 80;
    pub const GPIO1_COMBINED_16_31: u32 = 81;
    pub const GPIO2_COMBINED_0_15: u32 = 82;
    pub const GPIO2_COMBINED_16_31: u32 = 83;
    pub const GPT1: u32 = 100;
    pub const GPT2: u32 = 101;
    pub const PIT: u32 = 122;
}

// ---------------------------------------------------------------------------
// Aggregate
// ---------------------------------------------------------------------------

/// All peripheral state, routed by 16 KiB-aligned base address.
pub struct Peripherals {
    pub ccm: ccm::Ccm,
    pub ccm_analog: RawRegs,
    pub iomuxc: RawRegs,
    pub iomuxc_gpr: RawRegs,
    pub src: src::Src,
    pub gpc: RawRegs,
    pub snvs: RawRegs,
    pub dcdc: RawRegs,
    pub ocotp: RawRegs,
    pub pit: RawRegs,
    pub gpt: [gpt::Gpt; 2],
    pub wdog1: wdog::Wdog,
    pub wdog2: wdog::Wdog,
    pub rtwdog: wdog::Wdog,
    /// LPUART1..8 (index 0 = LPUART1). LPUART1 is the SwiftIO console.
    pub lpuart: [lpuart::LpUart; 8],
    /// GPIO1..9 (index 0 = GPIO1).
    pub gpio: [gpio::Gpio; 9],
    /// Fallback for every base without a model yet. One shared window: reads
    /// after writes to *different* unknown blocks may alias — acceptable
    /// until a real model lands.
    pub unknown: RawRegs,
    /// Log first access to each unknown base (default true).
    pub log_unknown: bool,
    warned_unknown: std::collections::BTreeSet<u32>,
}

impl Default for Peripherals {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripherals {
    pub fn new() -> Self {
        Self {
            ccm: ccm::Ccm::new(),
            // CCM_ANALOG reset: PLL lock bits are polled at boot. Seed the
            // ARM PLL / SYS PLL / USB PLL lock bits so spin-loops pass
            // (CCM_ANALOG_PLL_*_LOCK bit 31). Refined in the clock milestone.
            ccm_analog: RawRegs::new("ccm_analog")
                .with(0x000, 1 << 31) // PLL_ARM: LOCK
                .with(0x010, 1 << 31) // PLL_USB1: LOCK
                .with(0x020, 1 << 31) // PLL_USB2: LOCK
                .with(0x030, 1 << 31) // PLL_SYS: LOCK
                .with(0x070, 1 << 31) // PLL_AUDIO: LOCK
                .with(0x0A0, 1 << 31) // PLL_VIDEO: LOCK
                .with(0x0E0, 1 << 31), // PLL_ENET: LOCK
            iomuxc: RawRegs::new("iomuxc"),
            iomuxc_gpr: RawRegs::new("iomuxc_gpr"),
            src: src::Src::new(),
            gpc: RawRegs::new("gpc"),
            snvs: RawRegs::new("snvs"),
            dcdc: RawRegs::new("dcdc"),
            ocotp: RawRegs::new("ocotp"),
            pit: RawRegs::new("pit"),
            gpt: [gpt::Gpt::new(), gpt::Gpt::new()],
            wdog1: wdog::Wdog::new(wdog::Kind::Wdog),
            wdog2: wdog::Wdog::new(wdog::Kind::Wdog),
            rtwdog: wdog::Wdog::new(wdog::Kind::RtWdog),
            lpuart: std::array::from_fn(|i| lpuart::LpUart::new(i as u8 + 1)),
            gpio: std::array::from_fn(|i| gpio::Gpio::new(i as u8 + 1)),
            unknown: RawRegs::new("unknown"),
            log_unknown: true,
            warned_unknown: std::collections::BTreeSet::new(),
        }
    }

    /// Route a read to the owning block. `base` is 16 KiB-aligned; `offset`
    /// is `< 0x4000`.
    pub fn read(&mut self, addr: u32) -> u32 {
        let b = addr & !0x3FFF;
        let off = addr & 0x3FFF;
        match b {
            base::CCM => self.ccm.read(off),
            base::CCM_ANALOG => self.ccm_analog.read(off),
            base::IOMUXC => self.iomuxc.read(off),
            base::IOMUXC_GPR => self.iomuxc_gpr.read(off),
            base::SRC => self.src.read(off),
            base::GPC => self.gpc.read(off),
            base::SNVS => self.snvs.read(off),
            base::DCDC => self.dcdc.read(off),
            base::OCOTP => self.ocotp.read(off),
            base::PIT => self.pit.read(off),
            base::GPT1 => self.gpt[0].read(off),
            base::GPT2 => self.gpt[1].read(off),
            base::WDOG1 => self.wdog1.read(off),
            base::WDOG2 => self.wdog2.read(off),
            base::RTWDOG => self.rtwdog.read(off),
            base::LPUART1..=base::LPUART8 => self.lpuart[lpuart_index(b)].read(off),
            _ if is_gpio(b) => self.gpio[gpio_index(b)].read(off),
            _ => {
                self.warn_unknown("read", b);
                self.unknown.read(off)
            }
        }
    }

    /// Route a write to the owning block.
    pub fn write(&mut self, addr: u32, value: u32) {
        let b = addr & !0x3FFF;
        let off = addr & 0x3FFF;
        match b {
            base::CCM => self.ccm.write(off, value),
            base::CCM_ANALOG => self.ccm_analog.write(off, value),
            base::IOMUXC => self.iomuxc.write(off, value),
            base::IOMUXC_GPR => self.iomuxc_gpr.write(off, value),
            base::SRC => self.src.write(off, value),
            base::GPC => self.gpc.write(off, value),
            base::SNVS => self.snvs.write(off, value),
            base::DCDC => self.dcdc.write(off, value),
            base::OCOTP => self.ocotp.write(off, value),
            base::PIT => self.pit.write(off, value),
            base::GPT1 => self.gpt[0].write(off, value),
            base::GPT2 => self.gpt[1].write(off, value),
            base::WDOG1 => self.wdog1.write(off, value),
            base::WDOG2 => self.wdog2.write(off, value),
            base::RTWDOG => self.rtwdog.write(off, value),
            base::LPUART1..=base::LPUART8 => self.lpuart[lpuart_index(b)].write(off, value),
            _ if is_gpio(b) => self.gpio[gpio_index(b)].write(off, value),
            _ => {
                self.warn_unknown("write", b);
                self.unknown.write(off, value)
            }
        }
    }

    #[cold]
    fn warn_unknown(&mut self, kind: &str, base: u32) {
        if self.warned_unknown.insert(base) && self.log_unknown {
            eprintln!("[rt1060-rs] unmodeled peripheral {kind} at base {base:#010x}");
        }
    }

    /// Advance every time-driven peripheral by `cycles` core cycles. The
    /// bus core clock feeds the GPTs / PIT / watchdogs.
    pub fn tick(&mut self, cycles: u64) {
        self.gpt[0].tick(cycles);
        self.gpt[1].tick(cycles);
    }

    /// Assemble the level-sensitive external interrupt lines.
    pub fn irq_lines(&self) -> IrqMask {
        let mut m = IrqMask::ZERO;
        for (i, u) in self.lpuart.iter().enumerate() {
            if u.irq_pending() {
                // LPUART1..8 → IRQ 20..27 (contiguous).
                m.set(irq::LPUART1 + i as u32);
            }
        }
        if self.gpt[0].irq_pending() {
            m.set(irq::GPT1);
        }
        if self.gpt[1].irq_pending() {
            m.set(irq::GPT2);
        }
        // GPIO1..2 combined lines (0..15 / 16..31). GPIO3+ combined lines
        // land in the ROADMAP as their pin banks come online.
        if self.gpio[0].irq_pending_low() {
            m.set(irq::GPIO1_COMBINED_0_15);
        }
        if self.gpio[0].irq_pending_high() {
            m.set(irq::GPIO1_COMBINED_16_31);
        }
        if self.gpio[1].irq_pending_low() {
            m.set(irq::GPIO2_COMBINED_0_15);
        }
        if self.gpio[1].irq_pending_high() {
            m.set(irq::GPIO2_COMBINED_16_31);
        }
        m
    }
}

/// `true` if `base` is one of the nine GPIO controller windows.
#[inline]
fn is_gpio(base: u32) -> bool {
    matches!(
        base,
        base::GPIO1
            | base::GPIO2
            | base::GPIO3
            | base::GPIO4
            | base::GPIO5
            | base::GPIO6
            | base::GPIO7
            | base::GPIO8
            | base::GPIO9
    )
}

/// Index 0..8 for GPIO1..9 from a controller base.
#[inline]
fn gpio_index(base: u32) -> usize {
    match base {
        base::GPIO1 => 0,
        base::GPIO2 => 1,
        base::GPIO3 => 2,
        base::GPIO4 => 3,
        base::GPIO5 => 4,
        base::GPIO6 => 5,
        base::GPIO7 => 6,
        base::GPIO8 => 7,
        base::GPIO9 => 8,
        _ => unreachable!("gpio_index called on non-GPIO base"),
    }
}

/// Index 0..7 for LPUART1..8 from a controller base (16 KiB stride).
#[inline]
fn lpuart_index(base: u32) -> usize {
    ((base - base::LPUART1) / 0x4000) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rawregs_roundtrip() {
        let mut r = RawRegs::new("t").with(0x08, 0xDEAD_BEEF);
        assert_eq!(r.read(0x08), 0xDEAD_BEEF);
        r.write(0x10, 0x1234_5678);
        assert_eq!(r.read(0x10), 0x1234_5678);
    }

    #[test]
    fn base_indexing() {
        assert_eq!(lpuart_index(base::LPUART1), 0);
        assert_eq!(lpuart_index(base::LPUART8), 7);
        assert_eq!(gpio_index(base::GPIO1), 0);
        assert_eq!(gpio_index(base::GPIO9), 8);
        assert!(is_gpio(base::GPIO5));
        assert!(!is_gpio(base::CCM));
    }

    #[test]
    fn ccm_analog_pll_lock_seeded() {
        let mut p = Peripherals::new();
        p.log_unknown = false;
        // ARM PLL LOCK (bit 31) must read set so CLOCK_InitArmPll spin-loops
        // terminate (MIMXRT1062.h CCM_ANALOG_PLL_ARM_LOCK_MASK).
        assert_ne!(p.read(base::CCM_ANALOG) & (1 << 31), 0);
    }
}

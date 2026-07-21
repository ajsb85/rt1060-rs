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

pub mod adc;
pub mod analog;
pub mod ccm;
pub mod clocks;
pub mod edma;
pub mod flexcan;
pub mod flexspi;
pub mod gpio;
pub mod gpt;
pub mod lpi2c;
pub mod lpspi;
pub mod lpuart;
pub mod pit;
pub mod pwm;
pub mod qtmr;
pub mod sai;
pub mod semc;
pub mod src;
pub mod usb;
pub mod usdhc;
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
    pub const SAI1: u32 = 0x4038_4000;
    pub const SAI2: u32 = 0x4038_8000;
    pub const SAI3: u32 = 0x4038_C000;
    pub const CAN1: u32 = 0x401D_0000;
    pub const CAN2: u32 = 0x401D_4000;
    pub const CAN3: u32 = 0x401D_8000;
    pub const ADC1: u32 = 0x400C_4000;
    pub const ADC2: u32 = 0x400C_8000;
    pub const DCDC: u32 = 0x4008_0000;
    pub const DMA0: u32 = 0x400E_8000;
    pub const DMAMUX: u32 = 0x400E_C000;
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
    pub const SEMC: u32 = 0x402F_0000;
    pub const FLEXSPI: u32 = 0x402A_8000;
    pub const FLEXSPI2: u32 = 0x402A_4000;
    pub const USB: u32 = 0x402E_0000; // USB1 (off 0) + USB2 (off 0x200)
    pub const USDHC1: u32 = 0x402C_0000;
    pub const USDHC2: u32 = 0x402C_4000;
    pub const PWM1: u32 = 0x403D_C000;
    pub const PWM2: u32 = 0x403E_0000;
    pub const PWM3: u32 = 0x403E_4000;
    pub const PWM4: u32 = 0x403E_8000;
    pub const TMR1: u32 = 0x401D_C000;
    pub const TMR2: u32 = 0x401E_0000;
    pub const TMR3: u32 = 0x401E_4000;
    pub const TMR4: u32 = 0x401E_8000;
    pub const LPI2C1: u32 = 0x403F_0000;
    pub const LPI2C2: u32 = 0x403F_4000;
    pub const LPI2C3: u32 = 0x403F_8000;
    pub const LPI2C4: u32 = 0x403F_C000;
    pub const LPSPI1: u32 = 0x4039_4000;
    pub const LPSPI2: u32 = 0x4039_8000;
    pub const LPSPI3: u32 = 0x4039_C000;
    pub const LPSPI4: u32 = 0x403A_0000;
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
    pub const LPI2C1: u32 = 28;
    pub const LPI2C2: u32 = 29;
    pub const LPI2C3: u32 = 30;
    pub const LPI2C4: u32 = 31;
    pub const LPSPI1: u32 = 32;
    pub const LPSPI2: u32 = 33;
    pub const LPSPI3: u32 = 34;
    pub const LPSPI4: u32 = 35;
    pub const GPIO1_COMBINED_0_15: u32 = 80;
    pub const GPIO1_COMBINED_16_31: u32 = 81;
    pub const GPIO2_COMBINED_0_15: u32 = 82;
    pub const GPIO2_COMBINED_16_31: u32 = 83;
    pub const GPIO3_COMBINED_0_15: u32 = 84;
    pub const GPIO3_COMBINED_16_31: u32 = 85;
    pub const GPIO4_COMBINED_0_15: u32 = 86;
    pub const GPIO4_COMBINED_16_31: u32 = 87;
    pub const GPIO5_COMBINED_0_15: u32 = 88;
    pub const GPIO5_COMBINED_16_31: u32 = 89;
    /// GPIO6..9 share one combined line (MIMXRT1062.h `GPIO6_7_8_9_IRQn`).
    pub const GPIO6_7_8_9: u32 = 157;
    pub const ADC1: u32 = 67;
    pub const ADC2: u32 = 68;
    pub const GPT1: u32 = 100;
    pub const GPT2: u32 = 101;
    pub const PIT: u32 = 122;
    pub const TMR1: u32 = 133;
    pub const TMR2: u32 = 134;
    pub const TMR3: u32 = 135;
    pub const TMR4: u32 = 136;
    pub const USDHC1: u32 = 110;
    pub const USDHC2: u32 = 111;
    pub const USB_OTG1: u32 = 113;
    pub const USB_OTG2: u32 = 112;
    pub const CAN1: u32 = 36;
    pub const CAN2: u32 = 37;
    pub const CAN3: u32 = 154;
    pub const FLEXSPI: u32 = 108;
    pub const FLEXSPI2: u32 = 107;
    pub const SAI1: u32 = 56;
    pub const SAI2: u32 = 57;
    pub const SAI3: u32 = 58;
}

// ---------------------------------------------------------------------------
// Bus-event trace
// ---------------------------------------------------------------------------

/// Ordered log of firmware-driven bus activity for an embedding host (a
/// boardforge-style wiring engine). Recorded only when
/// [`Peripherals::trace_bus`] is set — the hot loop stays allocation-free by
/// default — and drained from [`Peripherals::bus_events`] between step
/// batches. Ordering across peripherals is preserved (a GPIO chip-select
/// write lands before the LPSPI `TDR` write it frames). Indices are the
/// peripheral array indices (`ctrl` 0 = GPIO1, `idx` 0 = LPUART1 / LPI2C1 /
/// LPSPI1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusEvent {
    /// GPIO DR/GDIR-affecting write (DR, GDIR, DR_SET/CLEAR/TOGGLE) with the
    /// post-write snapshot.
    GpioOut { ctrl: u8, dr: u32, gdir: u32 },
    /// IOMUXC window write (pad mux/config/daisy), offset within the window.
    IomuxcWrite { offset: u32, value: u32 },
    /// LPUART `DATA` write with the transmitter enabled.
    UartTx { idx: u8, byte: u8 },
    /// LPI2C `MTDR` command word (CMD in bits [10:8], DATA in [7:0]).
    I2cCmd { idx: u8, cmd: u32 },
    /// LPSPI `TDR` write while enabled: the MOSI word masked to the frame
    /// size (`bits` = `TCR.FRAMESZ` + 1, capped at 32).
    SpiTx { idx: u8, word: u32, bits: u8 },
}

// ---------------------------------------------------------------------------
// Aggregate
// ---------------------------------------------------------------------------

/// All peripheral state, routed by 16 KiB-aligned base address.
pub struct Peripherals {
    pub ccm: ccm::Ccm,
    pub ccm_analog: analog::CcmAnalog,
    pub edma: edma::Edma,
    pub dmamux: RawRegs,
    pub iomuxc: RawRegs,
    pub iomuxc_gpr: RawRegs,
    pub src: src::Src,
    pub gpc: RawRegs,
    pub snvs: RawRegs,
    pub dcdc: RawRegs,
    pub ocotp: RawRegs,
    pub pit: pit::Pit,
    pub semc: semc::Semc,
    /// LPI2C1..4 (index 0 = LPI2C1). SwiftIO `Id.I2C0` is LPI2C3.
    pub lpi2c: [lpi2c::LpI2c; 4],
    /// LPSPI1..4 (index 0 = LPSPI1). SwiftIO `Id.SPI0`/`SPI1` are LPSPI3/4.
    pub lpspi: [lpspi::LpSpi; 4],
    /// ADC1/2 (index 0 = ADC1).
    pub adc: [adc::Adc; 2],
    /// FlexPWM1..4 (index 0 = PWM1).
    pub pwm: [pwm::Pwm; 4],
    /// QTMR1..4 (index 0 = TMR1).
    pub qtmr: [qtmr::Qtmr; 4],
    /// USDHC1/2 (index 0 = USDHC1).
    pub usdhc: [usdhc::Usdhc; 2],
    /// USB1/USB2 OTG controllers (one shared window).
    pub usb: usb::Usb,
    /// FlexCAN1..3 (index 0 = CAN1).
    pub flexcan: [flexcan::FlexCan; 3],
    /// SAI1..3 (index 0 = SAI1).
    pub sai: [sai::Sai; 3],
    /// FlexSPI (index 0) + FlexSPI2 (index 1).
    pub flexspi: [flexspi::FlexSpi; 2],
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
    /// Record firmware bus writes into `bus_events` for an embedding host.
    pub trace_bus: bool,
    pub bus_events: Vec<BusEvent>,
    warned_unknown: std::collections::BTreeSet<u32>,
    /// Cached clock roots (Hz), recomputed whenever CCM/CCM_ANALOG is written.
    core_hz: u64,
    perclk_hz: u64,
    uart_hz: u64,
    /// Fractional carry for the core→domain cycle conversion in `tick`,
    /// one per clock domain: [0] PERCLK, [1] 24 MHz OSC, [2] 32.768 kHz LF.
    dom_frac: [u64; 3],
}

impl Default for Peripherals {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripherals {
    pub fn new() -> Self {
        let mut p = Self {
            ccm: ccm::Ccm::new(),
            ccm_analog: analog::CcmAnalog::new(),
            edma: edma::Edma::new(),
            dmamux: RawRegs::new("dmamux"),
            iomuxc: RawRegs::new("iomuxc"),
            iomuxc_gpr: RawRegs::new("iomuxc_gpr"),
            src: src::Src::new(),
            gpc: RawRegs::new("gpc"),
            snvs: RawRegs::new("snvs"),
            // DCDC: firmware raises VDD_SOC then spins on REG0.STS_DC_OK
            // (bit 31) before clocking to 600 MHz (clock_config.c). Seed it
            // set so the loop exits. A DCDC_Init that rewrites REG0 would need
            // a real model (ROADMAP).
            dcdc: RawRegs::new("dcdc").with(0x00, 1 << 31),
            ocotp: RawRegs::new("ocotp"),
            pit: pit::Pit::new(),
            semc: semc::Semc::new(),
            lpi2c: [
                lpi2c::LpI2c::new(1),
                lpi2c::LpI2c::new(2),
                lpi2c::LpI2c::new(3),
                lpi2c::LpI2c::new(4),
            ],
            lpspi: [
                lpspi::LpSpi::new(1),
                lpspi::LpSpi::new(2),
                lpspi::LpSpi::new(3),
                lpspi::LpSpi::new(4),
            ],
            adc: [adc::Adc::new(1), adc::Adc::new(2)],
            pwm: std::array::from_fn(|i| pwm::Pwm::new(i as u8 + 1)),
            qtmr: std::array::from_fn(|i| qtmr::Qtmr::new(i as u8 + 1)),
            usdhc: [usdhc::Usdhc::new(1), usdhc::Usdhc::new(2)],
            usb: usb::Usb::new(),
            flexcan: std::array::from_fn(|i| flexcan::FlexCan::new(i as u8 + 1)),
            sai: std::array::from_fn(|i| sai::Sai::new(i as u8 + 1)),
            flexspi: [flexspi::FlexSpi::new(1), flexspi::FlexSpi::new(2)],
            gpt: [gpt::Gpt::new(), gpt::Gpt::new()],
            wdog1: wdog::Wdog::new(wdog::Kind::Wdog),
            wdog2: wdog::Wdog::new(wdog::Kind::Wdog),
            rtwdog: wdog::Wdog::new(wdog::Kind::RtWdog),
            lpuart: std::array::from_fn(|i| lpuart::LpUart::new(i as u8 + 1)),
            gpio: std::array::from_fn(|i| gpio::Gpio::new(i as u8 + 1)),
            unknown: RawRegs::new("unknown"),
            log_unknown: true,
            trace_bus: false,
            bus_events: Vec::new(),
            warned_unknown: std::collections::BTreeSet::new(),
            core_hz: 1,
            perclk_hz: 1,
            uart_hz: 1,
            dom_frac: [0; 3],
        };
        p.refresh_clocks();
        p
    }

    /// Recompute the cached clock roots from the current CCM/CCM_ANALOG state.
    fn refresh_clocks(&mut self) {
        let c = clocks::Clocks::compute(&self.ccm, &self.ccm_analog);
        self.core_hz = c.core.max(1);
        self.perclk_hz = c.perclk.max(1);
        self.uart_hz = c.uart.max(1);
    }

    /// The current clock roots (Hz).
    pub fn clocks(&self) -> clocks::Clocks {
        clocks::Clocks::compute(&self.ccm, &self.ccm_analog)
    }

    /// Route a read to the owning block. `base` is 16 KiB-aligned; `offset`
    /// is `< 0x4000`.
    pub fn read(&mut self, addr: u32) -> u32 {
        let b = addr & !0x3FFF;
        let off = addr & 0x3FFF;
        match b {
            base::CCM => self.ccm.read(off),
            base::CCM_ANALOG => self.ccm_analog.read(off),
            base::DMA0 => self.edma.read32(off),
            base::DMAMUX => self.dmamux.read(off),
            base::IOMUXC => self.iomuxc.read(off),
            base::IOMUXC_GPR => self.iomuxc_gpr.read(off),
            base::SRC => self.src.read(off),
            base::GPC => self.gpc.read(off),
            base::SNVS => self.snvs.read(off),
            // DCDC REG0.STS_DC_OK (bit 31) is force-asserted so the VDD_SOC
            // ramp poll always exits, even if a driver rewrote REG0.
            base::DCDC if off == 0 => self.dcdc.read(off) | (1 << 31),
            base::DCDC => self.dcdc.read(off),
            base::OCOTP => self.ocotp.read(off),
            base::PIT => self.pit.read(off),
            base::SEMC => self.semc.read(off),
            base::USDHC1 => self.usdhc[0].read(off),
            base::USDHC2 => self.usdhc[1].read(off),
            base::USB => self.usb.read(off),
            base::CAN1 => self.flexcan[0].read(off),
            base::CAN2 => self.flexcan[1].read(off),
            base::CAN3 => self.flexcan[2].read(off),
            base::SAI1 => self.sai[0].read(off),
            base::SAI2 => self.sai[1].read(off),
            base::SAI3 => self.sai[2].read(off),
            base::FLEXSPI => self.flexspi[0].read(off),
            base::FLEXSPI2 => self.flexspi[1].read(off),
            base::LPI2C1 => self.lpi2c[0].read(off),
            base::LPI2C2 => self.lpi2c[1].read(off),
            base::LPI2C3 => self.lpi2c[2].read(off),
            base::LPI2C4 => self.lpi2c[3].read(off),
            base::LPSPI1 => self.lpspi[0].read(off),
            base::LPSPI2 => self.lpspi[1].read(off),
            base::LPSPI3 => self.lpspi[2].read(off),
            base::LPSPI4 => self.lpspi[3].read(off),
            base::ADC1 => self.adc[0].read(off),
            base::ADC2 => self.adc[1].read(off),
            base::PWM1 => self.pwm[0].read32(off),
            base::PWM2 => self.pwm[1].read32(off),
            base::PWM3 => self.pwm[2].read32(off),
            base::PWM4 => self.pwm[3].read32(off),
            base::TMR1 => self.qtmr[0].read32(off),
            base::TMR2 => self.qtmr[1].read32(off),
            base::TMR3 => self.qtmr[2].read32(off),
            base::TMR4 => self.qtmr[3].read32(off),
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
            base::DMA0 => self.edma.write32(off, value),
            base::DMAMUX => self.dmamux.write(off, value),
            base::IOMUXC => self.iomuxc.write(off, value),
            base::IOMUXC_GPR => self.iomuxc_gpr.write(off, value),
            base::SRC => self.src.write(off, value),
            base::GPC => self.gpc.write(off, value),
            base::SNVS => self.snvs.write(off, value),
            base::DCDC => self.dcdc.write(off, value),
            base::OCOTP => self.ocotp.write(off, value),
            base::PIT => self.pit.write(off, value),
            base::SEMC => self.semc.write(off, value),
            base::USDHC1 => self.usdhc[0].write(off, value),
            base::USDHC2 => self.usdhc[1].write(off, value),
            base::USB => self.usb.write(off, value),
            base::CAN1 => self.flexcan[0].write(off, value),
            base::CAN2 => self.flexcan[1].write(off, value),
            base::CAN3 => self.flexcan[2].write(off, value),
            base::SAI1 => self.sai[0].write(off, value),
            base::SAI2 => self.sai[1].write(off, value),
            base::SAI3 => self.sai[2].write(off, value),
            base::FLEXSPI => self.flexspi[0].write(off, value),
            base::FLEXSPI2 => self.flexspi[1].write(off, value),
            base::LPI2C1 => self.lpi2c[0].write(off, value),
            base::LPI2C2 => self.lpi2c[1].write(off, value),
            base::LPI2C3 => self.lpi2c[2].write(off, value),
            base::LPI2C4 => self.lpi2c[3].write(off, value),
            base::LPSPI1 => self.lpspi[0].write(off, value),
            base::LPSPI2 => self.lpspi[1].write(off, value),
            base::LPSPI3 => self.lpspi[2].write(off, value),
            base::LPSPI4 => self.lpspi[3].write(off, value),
            base::ADC1 => self.adc[0].write(off, value),
            base::ADC2 => self.adc[1].write(off, value),
            base::PWM1 => self.pwm[0].write32(off, value),
            base::PWM2 => self.pwm[1].write32(off, value),
            base::PWM3 => self.pwm[2].write32(off, value),
            base::PWM4 => self.pwm[3].write32(off, value),
            base::TMR1 => self.qtmr[0].write32(off, value),
            base::TMR2 => self.qtmr[1].write32(off, value),
            base::TMR3 => self.qtmr[2].write32(off, value),
            base::TMR4 => self.qtmr[3].write32(off, value),
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
        // A CCM / CCM_ANALOG write may have retuned the clock tree.
        if b == base::CCM || b == base::CCM_ANALOG {
            self.refresh_clocks();
        }
        if self.trace_bus {
            self.trace_bus_write(b, off, value);
        }
    }

    /// Append a [`BusEvent`] for a routed write. Runs post-dispatch so the
    /// GPIO snapshot observes the new DR/GDIR. Narrow (8/16-bit) peripheral
    /// writes funnel through [`Peripherals::write`], so they are traced too.
    fn trace_bus_write(&mut self, b: u32, off: u32, value: u32) {
        let ev = match b {
            _ if is_gpio(b) && matches!(off, 0x00 | 0x04 | 0x84 | 0x88 | 0x8C) => {
                let i = gpio_index(b);
                let g = &mut self.gpio[i];
                BusEvent::GpioOut {
                    ctrl: i as u8,
                    dr: g.read(0x00),
                    gdir: g.read(0x04),
                }
            }
            base::IOMUXC => BusEvent::IomuxcWrite { offset: off, value },
            base::LPUART1..=base::LPUART8 if off == 0x1C => {
                let idx = lpuart_index(b);
                if !self.lpuart[idx].tx_enabled() {
                    return;
                }
                BusEvent::UartTx {
                    idx: idx as u8,
                    byte: value as u8,
                }
            }
            base::LPI2C1..=base::LPI2C4 if off == 0x60 => BusEvent::I2cCmd {
                idx: ((b - base::LPI2C1) / 0x4000) as u8,
                cmd: value & 0x7FF,
            },
            base::LPSPI1..=base::LPSPI4 if off == 0x64 => {
                let idx = ((b - base::LPSPI1) / 0x4000) as usize;
                let s = &self.lpspi[idx];
                if !s.enabled() {
                    return;
                }
                let bits = s.frame_bits().min(32);
                let mask = if bits >= 32 {
                    u32::MAX
                } else {
                    (1u32 << bits) - 1
                };
                BusEvent::SpiTx {
                    idx: idx as u8,
                    word: value & mask,
                    bits: bits as u8,
                }
            }
            _ => return,
        };
        self.bus_events.push(ev);
    }

    // --- width-accurate narrow access ---------------------------------------
    //
    // The eDMA TCD packs two 16-bit fields per word, so byte/halfword accesses
    // to its window (`DMA0`) must land on the exact addressed bytes rather than
    // the replicate-to-word path ordinary word registers use. The bus routes
    // 8/16-bit peripheral access through these methods.

    /// Index of the FlexPWM instance for a base, if it is one.
    #[inline]
    fn pwm_index(base: u32) -> Option<usize> {
        match base {
            base::PWM1 => Some(0),
            base::PWM2 => Some(1),
            base::PWM3 => Some(2),
            base::PWM4 => Some(3),
            _ => None,
        }
    }

    /// Index of the QTMR instance for a base, if it is one.
    #[inline]
    fn qtmr_index(base: u32) -> Option<usize> {
        match base {
            base::TMR1 => Some(0),
            base::TMR2 => Some(1),
            base::TMR3 => Some(2),
            base::TMR4 => Some(3),
            _ => None,
        }
    }

    pub fn read8(&mut self, addr: u32) -> u8 {
        let (b, off) = (addr & !0x3FFF, addr & 0x3FFF);
        if b == base::DMA0 {
            self.edma.read8(off)
        } else if let Some(i) = Self::pwm_index(b) {
            self.pwm[i].read8(off)
        } else if let Some(i) = Self::qtmr_index(b) {
            self.qtmr[i].read8(off)
        } else {
            (self.read(addr & !0x3) >> ((addr & 0x3) * 8)) as u8
        }
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        let (b, off) = (addr & !0x3FFF, addr & 0x3FFF);
        if b == base::DMA0 {
            self.edma.read16(off)
        } else if let Some(i) = Self::pwm_index(b) {
            self.pwm[i].read16(off)
        } else if let Some(i) = Self::qtmr_index(b) {
            self.qtmr[i].read16(off)
        } else {
            (self.read(addr & !0x3) >> ((addr & 0x2) * 8)) as u16
        }
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        let (b, off) = (addr & !0x3FFF, addr & 0x3FFF);
        if b == base::DMA0 {
            self.edma.write8(off, value);
        } else if let Some(i) = Self::pwm_index(b) {
            self.pwm[i].write8(off, value);
        } else if let Some(i) = Self::qtmr_index(b) {
            self.qtmr[i].write8(off, value);
        } else {
            let v = u32::from(value);
            self.write(addr & !0x3, v << 24 | v << 16 | v << 8 | v);
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        let (b, off) = (addr & !0x3FFF, addr & 0x3FFF);
        if b == base::DMA0 {
            self.edma.write16(off, value);
        } else if let Some(i) = Self::pwm_index(b) {
            self.pwm[i].write16(off, value);
        } else if let Some(i) = Self::qtmr_index(b) {
            self.qtmr[i].write16(off, value);
        } else {
            let v = u32::from(value);
            self.write(addr & !0x3, v << 16 | v);
        }
    }

    #[cold]
    fn warn_unknown(&mut self, kind: &str, base: u32) {
        if self.warned_unknown.insert(base) && self.log_unknown {
            eprintln!("[rt1060-rs] unmodeled peripheral {kind} at base {base:#010x}");
        }
    }

    /// Advance every time-driven peripheral by `cycles` **core** cycles. The
    /// GPTs and PIT run on PERCLK, so convert into the PERCLK domain with a
    /// fractional carry to avoid drift when `perclk_hz` doesn't divide
    /// `core_hz` evenly.
    pub fn tick(&mut self, cycles: u64) {
        // Convert `cycles` core cycles into each clock domain, carrying the
        // remainder in `dom_frac` so no ticks are lost to integer division.
        let core = self.core_hz;
        let domain = |freq: u64, frac: &mut u64| -> u64 {
            let total = *frac + cycles.saturating_mul(freq);
            let ticks = total / core;
            *frac = total % core;
            ticks
        };
        let [f0, f1, f2] = &mut self.dom_frac;
        let perclk_ticks = domain(self.perclk_hz, f0);
        let osc_ticks = domain(clocks::OSC24M, f1);
        let lf_ticks = domain(32_768, f2);
        // Each GPT counts in the domain its CR.CLKSRC selects.
        for g in &mut self.gpt {
            let t = match g.clksrc() {
                0 => 0,
                5 => osc_ticks,    // 24 MHz crystal
                4 => lf_ticks,     // 32.768 kHz low-freq
                _ => perclk_ticks, // 1 = PERCLK (2/3 approximated as PERCLK)
            };
            g.tick(t);
        }
        self.pit.tick(perclk_ticks);
        for t in &mut self.qtmr {
            t.tick(perclk_ticks);
        }
    }

    /// The current cached clock roots (Hz): `(core, perclk, uart)`.
    pub fn clock_hz(&self) -> (u64, u64, u64) {
        (self.core_hz, self.perclk_hz, self.uart_hz)
    }

    /// Level of the DMA request line for a given DMAMUX `source` number
    /// (MIMXRT1062.h `_dma_request_source`), used by the eDMA hardware-request
    /// service. Unmodeled sources read low.
    pub fn dma_request_level(&self, source: u32) -> bool {
        match source {
            2 => self.lpuart[0].dma_tx_request(),
            3 => self.lpuart[0].dma_rx_request(),
            66 => self.lpuart[1].dma_tx_request(),
            67 => self.lpuart[1].dma_rx_request(),
            13 => self.lpspi[0].dma_rx_request(),
            14 => self.lpspi[0].dma_tx_request(),
            77 => self.lpspi[1].dma_rx_request(),
            78 => self.lpspi[1].dma_tx_request(),
            17 => self.lpi2c[0].dma_request(),
            81 => self.lpi2c[1].dma_request(),
            24 => self.adc[0].dma_request(),
            88 => self.adc[1].dma_request(),
            _ => false,
        }
    }

    /// Whether any eDMA channel has a hardware request enabled (cheap gate for
    /// the per-step service).
    pub fn edma_hw_enabled(&self) -> bool {
        self.edma.hw_enabled()
    }

    /// Read a DMAMUX `CHCFG[ch]` (source routing) value.
    pub fn dmamux_chcfg(&self, ch: usize) -> u32 {
        self.dmamux.read(ch as u32 * 4)
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
        if self.pit.irq_pending() {
            m.set(irq::PIT);
        }
        for (i, t) in self.qtmr.iter().enumerate() {
            if t.irq_pending() {
                m.set(irq::TMR1 + i as u32); // TMR1..4 = IRQ 133..136
            }
        }
        if self.usdhc[0].irq_pending() {
            m.set(irq::USDHC1);
        }
        if self.usdhc[1].irq_pending() {
            m.set(irq::USDHC2);
        }
        if self.usb.irq_pending(0) {
            m.set(irq::USB_OTG1);
        }
        if self.usb.irq_pending(1) {
            m.set(irq::USB_OTG2);
        }
        for (i, c) in self.flexcan.iter().enumerate() {
            if c.irq_pending() {
                m.set([irq::CAN1, irq::CAN2, irq::CAN3][i]);
            }
        }
        for (i, s) in self.sai.iter().enumerate() {
            if s.irq_pending() {
                m.set([irq::SAI1, irq::SAI2, irq::SAI3][i]);
            }
        }
        if self.flexspi[0].irq_pending() {
            m.set(irq::FLEXSPI);
        }
        if self.flexspi[1].irq_pending() {
            m.set(irq::FLEXSPI2);
        }
        if self.lpi2c[0].irq_pending() {
            m.set(irq::LPI2C1);
        }
        if self.lpi2c[1].irq_pending() {
            m.set(irq::LPI2C2);
        }
        if self.lpi2c[2].irq_pending() {
            m.set(irq::LPI2C3);
        }
        if self.lpi2c[3].irq_pending() {
            m.set(irq::LPI2C4);
        }
        if self.lpspi[0].irq_pending() {
            m.set(irq::LPSPI1);
        }
        if self.lpspi[1].irq_pending() {
            m.set(irq::LPSPI2);
        }
        if self.lpspi[2].irq_pending() {
            m.set(irq::LPSPI3);
        }
        if self.lpspi[3].irq_pending() {
            m.set(irq::LPSPI4);
        }
        if self.adc[0].irq_pending() {
            m.set(irq::ADC1);
        }
        if self.adc[1].irq_pending() {
            m.set(irq::ADC2);
        }
        let dma = self.edma.irq_lines16();
        for ch in 0..16u32 {
            if dma & (1 << ch) != 0 {
                m.set(ch); // DMAn_DMAn+16 = IRQ n
            }
        }
        // GPIO1..5 combined lines (0..15 / 16..31 pairs, IRQ 80..89);
        // GPIO6..9 share IRQ 157 (MIMXRT1062.h `GPIO_COMBINED_LOW_IRQS`).
        for (i, g) in self.gpio.iter().enumerate() {
            let (lo, hi) = (g.irq_pending_low(), g.irq_pending_high());
            if i < 5 {
                if lo {
                    m.set(irq::GPIO1_COMBINED_0_15 + 2 * i as u32);
                }
                if hi {
                    m.set(irq::GPIO1_COMBINED_16_31 + 2 * i as u32);
                }
            } else if lo || hi {
                m.set(irq::GPIO6_7_8_9);
            }
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
    fn bus_event_trace_orders_firmware_writes() {
        let mut p = Peripherals::new();
        p.log_unknown = false;
        p.trace_bus = true;
        p.write(base::GPIO1 + 0x04, 1 << 9); // GDIR
        p.write(base::LPUART1 + 0x18, 1 << 19); // CTRL.TE — no event
        p.write(base::LPUART1 + 0x1C, u32::from(b'A')); // DATA
        p.write(base::LPSPI3 + 0x10, 1); // CR.MEN — no event
        p.write(base::LPSPI3 + 0x60, 7); // TCR: 8-bit frames — no event
        p.write(base::LPSPI3 + 0x64, 0x1A5); // TDR → masked to 0xA5
        p.write(base::LPI2C3 + 0x60, (0b100 << 8) | 0x3A); // START
        p.write(base::IOMUXC + 0x14, 5);
        p.write(base::GPIO1 + 0x84, 1 << 9); // DR_SET
        assert_eq!(
            p.bus_events,
            vec![
                BusEvent::GpioOut {
                    ctrl: 0,
                    dr: 0,
                    gdir: 1 << 9
                },
                BusEvent::UartTx { idx: 0, byte: b'A' },
                BusEvent::SpiTx {
                    idx: 2,
                    word: 0xA5,
                    bits: 8
                },
                BusEvent::I2cCmd {
                    idx: 2,
                    cmd: (0b100 << 8) | 0x3A
                },
                BusEvent::IomuxcWrite {
                    offset: 0x14,
                    value: 5
                },
                BusEvent::GpioOut {
                    ctrl: 0,
                    dr: 1 << 9,
                    gdir: 1 << 9
                },
            ]
        );
    }

    #[test]
    fn bus_event_trace_off_by_default() {
        let mut p = Peripherals::new();
        p.log_unknown = false;
        p.write(base::GPIO1 + 0x04, 1 << 9);
        p.write(base::LPUART1 + 0x18, 1 << 19);
        p.write(base::LPUART1 + 0x1C, u32::from(b'A'));
        assert!(p.bus_events.is_empty());
    }

    #[test]
    fn gpio_input_edges_reach_the_combined_irq_lines() {
        let mut p = Peripherals::new();
        p.log_unknown = false;
        // GPIO3 pin 2: rising edge (ICR1 cfg 0b10) + unmask → IRQ 84.
        p.write(base::GPIO3 + 0x0C, 0b10 << 4);
        p.write(base::GPIO3 + 0x14, 1 << 2);
        p.gpio[2].set_input(2, true);
        assert!(p.irq_lines().test(irq::GPIO3_COMBINED_0_15));
        // GPIO7 pin 3: any edge (EDGE_SEL) + unmask → the shared IRQ 157.
        p.write(base::GPIO7 + 0x1C, 1 << 3);
        p.write(base::GPIO7 + 0x14, 1 << 3);
        p.gpio[6].set_input(3, true);
        assert!(p.irq_lines().test(irq::GPIO6_7_8_9));
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

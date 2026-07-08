// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Memory regions and the system bus for the i.MX RT1060 (SwiftIO Micro).
//!
//! Ported from the rp2350-rs / mg24-rs `memory/mod.rs` shape (region fast
//! path, warn-once unmapped logging, narrow-IO lane conventions); the map is
//! the i.MX RT1060's (MIMXRT1062.h + the SwiftIO Micro linker script
//! `../mm-sdk/boards/SwiftIOMicro/linker/sdram.ld`):
//!
//! | region          | base        | size      | notes                        |
//! |-----------------|-------------|-----------|------------------------------|
//! | ITCM            | 0x0000_0000 | 512 KiB   | FlexRAM (SwiftIO uses 128 K) |
//! | Boot ROM        | 0x0020_0000 | 96 KiB    | mask ROM (stubbed)           |
//! | DTCM            | 0x2000_0000 | 512 KiB   | FlexRAM (SwiftIO uses 128 K) |
//! | OCRAM           | 0x2020_0000 | 1 MiB     | 512 K dedicated + FlexRAM    |
//! | FlexSPI1 (NOR)  | 0x6000_0000 | 16 MiB    | XIP flash; erased reads 0xFF |
//! | SEMC SDRAM      | 0x8000_0000 | 32 MiB    | MadMachine image runs here   |
//!
//! Peripheral space (0x4000_0000..0x4400_0000, plus the 0x4200_0000 GPIO
//! island) routes to [`crate::peripherals::Peripherals`]; the PPB
//! (0xE000_0000+) never reaches the bus — the core intercepts it.

use crate::cortex_m::IrqMask;
use crate::peripherals::Peripherals;

/// Region bases/sizes (MIMXRT1062.h + SwiftIO Micro `sdram.ld`).
pub mod map {
    pub const ITCM_BASE: u32 = 0x0000_0000;
    pub const ITCM_SIZE: usize = 0x0008_0000; // 512 KiB max FlexRAM window
    pub const DTCM_BASE: u32 = 0x2000_0000;
    pub const DTCM_SIZE: usize = 0x0008_0000; // 512 KiB max FlexRAM window
    pub const OCRAM_BASE: u32 = 0x2020_0000;
    pub const OCRAM_SIZE: usize = 0x0010_0000; // 1 MiB (dedicated + FlexRAM)
    /// FlexSPI1 XIP NOR flash. SwiftIO Micro carries 16 MiB.
    pub const FLASH_BASE: u32 = 0x6000_0000;
    pub const FLASH_SIZE: usize = 0x0100_0000; // 16 MiB
    /// SEMC-attached SDRAM. SwiftIO Micro carries 32 MiB; the MadMachine
    /// user image loads and runs from here (`IMAGE_LOAD_ADDRESS`).
    pub const SDRAM_BASE: u32 = 0x8000_0000;
    pub const SDRAM_SIZE: usize = 0x0200_0000; // 32 MiB

    /// Peripheral space covered by the AIPS routing (incl. the high-speed
    /// GPIO island at 0x4200_0000).
    pub const PERIPH_LO: u32 = 0x4000_0000;
    pub const PERIPH_HI: u32 = 0x4400_0000;
}

pub struct SystemBus {
    pub itcm: Box<[u8]>,
    pub dtcm: Box<[u8]>,
    pub ocram: Box<[u8]>,
    /// 16 MiB FlexSPI NOR; erased state is 0xFF like the real article.
    pub flash: Box<[u8]>,
    /// 32 MiB SDRAM.
    pub sdram: Box<[u8]>,
    pub periph: Peripherals,
    /// Log unmapped accesses to stderr, once per 256-byte bucket.
    pub log_unmapped: bool,
    /// Trace peripheral writes to stderr (env `RT1060_TRACE` set).
    pub trace_writes: bool,
    /// Trace peripheral reads too (env `RT1060_TRACE` = `all`/contains `read`).
    pub trace_reads: bool,
    warned: std::collections::BTreeSet<u32>,
}

impl Default for SystemBus {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemBus {
    pub fn new() -> Self {
        // `RT1060_TRACE` (any value) traces peripheral writes; `=all` (or a
        // value containing `read`) traces reads too.
        let trace = std::env::var("RT1060_TRACE").ok();
        Self {
            itcm: vec![0; map::ITCM_SIZE].into_boxed_slice(),
            dtcm: vec![0; map::DTCM_SIZE].into_boxed_slice(),
            ocram: vec![0; map::OCRAM_SIZE].into_boxed_slice(),
            flash: vec![0xFF; map::FLASH_SIZE].into_boxed_slice(),
            sdram: vec![0; map::SDRAM_SIZE].into_boxed_slice(),
            periph: Peripherals::new(),
            log_unmapped: true,
            trace_writes: trace.is_some(),
            trace_reads: trace
                .as_deref()
                .map(|v| v == "all" || v.contains("read"))
                .unwrap_or(false),
            warned: std::collections::BTreeSet::new(),
        }
    }

    /// Copy a loaded image into the backing regions. Bytes outside every
    /// region are dropped with a warn-once unmapped log.
    pub fn load_segments(&mut self, image: &crate::loader::LoadedImage) {
        for seg in &image.segments {
            let mut addr = seg.addr;
            let mut rest = &seg.data[..];
            while !rest.is_empty() {
                let Some((region, off)) = self.region(addr) else {
                    self.unmapped("load", addr);
                    addr = addr.wrapping_add(1);
                    rest = &rest[1..];
                    continue;
                };
                let n = rest.len().min(region.len() - off);
                region[off..off + n].copy_from_slice(&rest[..n]);
                addr = addr.wrapping_add(n as u32);
                rest = &rest[n..];
            }
        }
    }

    #[cold]
    fn unmapped(&mut self, kind: &str, addr: u32) {
        if self.warned.insert(addr >> 8) && self.log_unmapped {
            eprintln!("[rt1060-rs] unmapped {kind} at {addr:#010x}");
        }
    }

    /// Resolve an address to its RAM-like backing region and offset.
    #[inline(always)]
    fn region(&mut self, addr: u32) -> Option<(&mut [u8], usize)> {
        // Fast path order: SDRAM (code+data) and flash dominate real traffic.
        if addr.wrapping_sub(map::SDRAM_BASE) < map::SDRAM_SIZE as u32 {
            return Some((&mut self.sdram, (addr - map::SDRAM_BASE) as usize));
        }
        if addr.wrapping_sub(map::FLASH_BASE) < map::FLASH_SIZE as u32 {
            return Some((&mut self.flash, (addr - map::FLASH_BASE) as usize));
        }
        if addr.wrapping_sub(map::OCRAM_BASE) < map::OCRAM_SIZE as u32 {
            return Some((&mut self.ocram, (addr - map::OCRAM_BASE) as usize));
        }
        // DTCM before ITCM: DTCM base (0x2000_0000) is distinct, but OCRAM
        // (0x2020_0000) sits above it — the OCRAM check above already ran.
        if addr.wrapping_sub(map::DTCM_BASE) < map::DTCM_SIZE as u32 {
            return Some((&mut self.dtcm, (addr - map::DTCM_BASE) as usize));
        }
        if addr.wrapping_sub(map::ITCM_BASE) < map::ITCM_SIZE as u32 {
            return Some((&mut self.itcm, (addr - map::ITCM_BASE) as usize));
        }
        None
    }

    #[inline(always)]
    fn ram_slice(&mut self, addr: u32, len: usize) -> Option<&mut [u8]> {
        let (region, off) = self.region(addr)?;
        region.get_mut(off..off + len)
    }

    #[inline(always)]
    fn is_periph(addr: u32) -> bool {
        (map::PERIPH_LO..map::PERIPH_HI).contains(&addr)
    }

    fn periph_read(&mut self, addr: u32) -> u32 {
        if Self::is_periph(addr) {
            let v = self.periph.read(addr);
            if self.trace_reads {
                self.trace('R', addr, v);
            }
            v
        } else {
            self.unmapped("read", addr);
            0
        }
    }

    fn periph_write(&mut self, addr: u32, value: u32) {
        if Self::is_periph(addr) {
            if self.trace_writes {
                self.trace('W', addr, value);
            }
            self.periph.write(addr, value);
            self.dma_trigger(addr);
        } else {
            self.unmapped("write", addr);
        }
    }

    /// One peripheral-access trace line, tagged by 16 KiB base.
    #[cold]
    fn trace(&self, kind: char, addr: u32, value: u32) {
        eprintln!(
            "[rt1060-rs trace] {kind} {addr:#010x} = {value:#010x} (base {:#010x})",
            addr & !0x3FFF
        );
    }

    /// A write into the eDMA window may have set `TCD.CSR.START` / `SSRT` —
    /// service the engine at strobe time (like the mg24-rs LDMA), moving data
    /// through the full bus. The engine is swapped out so it can borrow the
    /// bus for descriptor + FIFO access.
    #[inline]
    fn dma_trigger(&mut self, addr: u32) {
        if addr & !0x3FFF == crate::peripherals::base::DMA0 {
            self.edma_service();
        }
    }

    /// One eDMA service pass for software-started channels (empty request
    /// slice). Called at DMA-write strobe time.
    pub fn edma_service(&mut self) {
        let mut engine = std::mem::take(&mut self.periph.edma);
        engine.service(self, &[]);
        self.periph.edma = engine;
    }

    /// One eDMA service pass driven by hardware requests: for each channel,
    /// DMAMUX `CHCFG[ch]` (ENBL + source) selects a peripheral request line
    /// whose level (with `ERQ[ch]`) triggers a minor loop. Cheap-gated on any
    /// channel having its request enabled; called once per SoC step.
    pub fn edma_service_hw(&mut self) {
        if !self.periph.edma_hw_enabled() {
            return;
        }
        let mut req = [false; 32];
        for (ch, r) in req.iter_mut().enumerate() {
            let chcfg = self.periph.dmamux_chcfg(ch);
            if chcfg & 0x8000_0000 != 0 {
                *r = self.periph.dma_request_level(chcfg & 0x7F);
            }
        }
        let mut engine = std::mem::take(&mut self.periph.edma);
        engine.service(self, &req);
        self.periph.edma = engine;
    }

    // --- accessors (little-endian; unaligned OK inside RAM-like regions) ----

    pub fn read32(&mut self, addr: u32) -> u32 {
        if let Some(bytes) = self.ram_slice(addr, 4) {
            return u32::from_le_bytes(bytes.try_into().unwrap());
        }
        self.periph_read(addr)
    }

    pub fn write32(&mut self, addr: u32, value: u32) {
        if let Some(bytes) = self.ram_slice(addr, 4) {
            bytes.copy_from_slice(&value.to_le_bytes());
            return;
        }
        self.periph_write(addr, value);
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        if let Some(bytes) = self.ram_slice(addr, 2) {
            return u16::from_le_bytes(bytes.try_into().unwrap());
        }
        // Peripheral narrow reads are width-aware (see Peripherals::read16 —
        // ordinary word registers extract lanes; the eDMA TCD is byte-exact).
        if Self::is_periph(addr) {
            self.periph.read16(addr)
        } else {
            self.unmapped("read", addr);
            0
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        if let Some(bytes) = self.ram_slice(addr, 2) {
            bytes.copy_from_slice(&value.to_le_bytes());
            return;
        }
        if Self::is_periph(addr) {
            if self.trace_writes {
                self.trace('w', addr, u32::from(value));
            }
            self.periph.write16(addr, value);
            self.dma_trigger(addr);
        } else {
            self.unmapped("write", addr);
        }
    }

    pub fn read8(&mut self, addr: u32) -> u8 {
        if let Some(bytes) = self.ram_slice(addr, 1) {
            return bytes[0];
        }
        if Self::is_periph(addr) {
            self.periph.read8(addr)
        } else {
            self.unmapped("read", addr);
            0
        }
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        if let Some(bytes) = self.ram_slice(addr, 1) {
            bytes[0] = value;
            return;
        }
        if Self::is_periph(addr) {
            if self.trace_writes {
                self.trace('b', addr, u32::from(value));
            }
            self.periph.write8(addr, value);
            self.dma_trigger(addr);
        } else {
            self.unmapped("write", addr);
        }
    }
}

/// The eDMA moves data through full bus dispatch — descriptors in SRAM,
/// peripheral FIFO registers as endpoints — so the bus *is* the DMA's memory.
impl crate::peripherals::edma::DmaMem for SystemBus {
    fn read8(&mut self, addr: u32) -> u8 {
        SystemBus::read8(self, addr)
    }
    fn read16(&mut self, addr: u32) -> u16 {
        SystemBus::read16(self, addr)
    }
    fn read32(&mut self, addr: u32) -> u32 {
        SystemBus::read32(self, addr)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        SystemBus::write8(self, addr, value)
    }
    fn write16(&mut self, addr: u32, value: u16) {
        SystemBus::write16(self, addr, value)
    }
    fn write32(&mut self, addr: u32, value: u32) {
        SystemBus::write32(self, addr, value)
    }
}

impl crate::cortex_m::Bus for SystemBus {
    #[inline(always)]
    fn read8(&mut self, addr: u32) -> u8 {
        SystemBus::read8(self, addr)
    }
    #[inline(always)]
    fn read16(&mut self, addr: u32) -> u16 {
        SystemBus::read16(self, addr)
    }
    #[inline(always)]
    fn read32(&mut self, addr: u32) -> u32 {
        SystemBus::read32(self, addr)
    }
    #[inline(always)]
    fn write8(&mut self, addr: u32, value: u8) {
        SystemBus::write8(self, addr, value)
    }
    #[inline(always)]
    fn write16(&mut self, addr: u32, value: u16) {
        SystemBus::write16(self, addr, value)
    }
    #[inline(always)]
    fn write32(&mut self, addr: u32, value: u32) {
        SystemBus::write32(self, addr, value)
    }
    #[inline(always)]
    fn irq_lines(&mut self) -> IrqMask {
        self.periph.irq_lines()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::base;

    fn quiet_bus() -> SystemBus {
        let mut bus = SystemBus::new();
        bus.log_unmapped = false;
        bus.periph.log_unknown = false;
        bus
    }

    #[test]
    fn flash_reads_erased_then_sdram_roundtrips() {
        let mut bus = quiet_bus();
        assert_eq!(bus.read32(map::FLASH_BASE), 0xFFFF_FFFF);
        assert_eq!(bus.read8(map::FLASH_BASE + 0x1234), 0xFF);
        // SDRAM: little-endian lanes + unaligned.
        bus.write32(map::SDRAM_BASE + 0x100, 0xDEAD_BEEF);
        assert_eq!(bus.read32(map::SDRAM_BASE + 0x100), 0xDEAD_BEEF);
        assert_eq!(bus.read8(map::SDRAM_BASE + 0x100), 0xEF);
        assert_eq!(bus.read16(map::SDRAM_BASE + 0x102), 0xDEAD);
        bus.write32(map::SDRAM_BASE + 0x201, 0xA5A5_5A5A);
        assert_eq!(bus.read32(map::SDRAM_BASE + 0x201), 0xA5A5_5A5A);
    }

    #[test]
    fn tcm_and_ocram_distinct_backings() {
        let mut bus = quiet_bus();
        bus.write32(map::ITCM_BASE + 0x10, 0x1111_1111);
        bus.write32(map::DTCM_BASE + 0x10, 0x2222_2222);
        bus.write32(map::OCRAM_BASE + 0x10, 0x3333_3333);
        assert_eq!(bus.read32(map::ITCM_BASE + 0x10), 0x1111_1111);
        assert_eq!(bus.read32(map::DTCM_BASE + 0x10), 0x2222_2222);
        assert_eq!(bus.read32(map::OCRAM_BASE + 0x10), 0x3333_3333);
    }

    #[test]
    fn lpuart1_tx_through_bus_reaches_host() {
        let mut bus = quiet_bus();
        // CTRL @ +0x18: TE (bit 19); DATA @ +0x1C.
        bus.write32(base::LPUART1 + 0x18, 1 << 19);
        bus.write32(base::LPUART1 + 0x1C, u32::from(b'O'));
        bus.write32(base::LPUART1 + 0x1C, u32::from(b'K'));
        assert_eq!(bus.periph.lpuart[0].take_output_string(), "OK");
    }

    #[test]
    fn gpio1_dr_set_visible_on_bus() {
        let mut bus = quiet_bus();
        bus.write32(base::GPIO1 + 0x04, 0xFFFF_FFFF); // GDIR: outputs
        bus.write32(base::GPIO1 + 0x84, 1 << 9); // DR_SET pin 9
        assert!(bus.periph.gpio[0].output(9));
        assert_ne!(bus.read32(base::GPIO1) & (1 << 9), 0);
    }

    #[test]
    fn irq_lines_track_lpuart1_rx() {
        use crate::cortex_m::Bus;
        let mut bus = quiet_bus();
        bus.write32(base::LPUART1 + 0x18, (1 << 18) | (1 << 21)); // RE|RIE
        assert!(bus.irq_lines().is_zero());
        bus.periph.lpuart[0].rx_push(b'z');
        assert!(bus.irq_lines().test(20), "LPUART1 = IRQ 20");
    }

    #[test]
    fn tracing_does_not_alter_behavior() {
        // With write tracing on, register access must still work identically
        // (the trace is a pure side channel to stderr).
        let mut bus = quiet_bus();
        bus.trace_writes = true;
        bus.trace_reads = true;
        bus.write32(base::GPIO1 + 0x04, 0xFFFF_FFFF); // GDIR
        bus.write32(base::GPIO1 + 0x84, 1 << 3); // DR_SET pin 3
        assert!(bus.periph.gpio[0].output(3));
        assert_eq!(bus.read32(base::GPIO1), 1 << 3);
    }

    #[test]
    fn unmapped_reads_zero_when_gagged() {
        let mut bus = quiet_bus();
        assert_eq!(bus.read32(0x0100_0000), 0); // hole below flash
        assert_eq!(bus.read32(0x9000_0000), 0); // past SDRAM
        bus.write32(0x0100_0000, 42); // dropped, no panic
        assert_eq!(bus.read32(0x0100_0000), 0);
    }
}

// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Watchdogs — WDOG1/WDOG2 (16-bit WDOG, RM §60) and RTWDOG (32-bit, RM §61).
//!
//! Stored-readback for now: firmware either disables the watchdog or services
//! it, and the model never bites (no timeout reset yet — that lands in the
//! ROADMAP alongside the SoC reset path). WDOG uses 16-bit registers; RTWDOG
//! uses a 32-bit unlock (CS/CNT/TOVAL/WIN) sequence. Both simply accept and
//! read back their configuration here.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// WDOG1/WDOG2: 16-bit register interface.
    Wdog,
    /// RTWDOG: 32-bit register interface with a refresh/unlock protocol.
    RtWdog,
}

pub struct Wdog {
    kind: Kind,
    regs: [u32; 8],
}

impl Wdog {
    pub fn new(kind: Kind) -> Self {
        let mut regs = [0u32; 8];
        match kind {
            // WDOG WCR (0x00) reset value: WDE clear, WDT/SRS defaults.
            Kind::Wdog => regs[0] = 0x0030,
            // RTWDOG CS (0x00) reset: enabled, 32-bit, LPO clock (RM §61.5.1
            // RTWDOG_CS = 0x0000_2980). TOVAL reset 0x0400.
            Kind::RtWdog => {
                regs[0] = 0x0000_2980;
                regs[2] = 0x0000_0400;
            }
        }
        Self { kind, regs }
    }

    #[inline]
    fn idx(&self, offset: u32) -> usize {
        match self.kind {
            // WDOG registers are 2-byte strided (WCR 0x00, WSR 0x02, …).
            Kind::Wdog => (offset >> 1) as usize & 7,
            // RTWDOG registers are 4-byte strided (CS 0x00, CNT 0x04, …).
            Kind::RtWdog => (offset >> 2) as usize & 7,
        }
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        self.regs[self.idx(offset)]
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        // WSR (WDOG 0x02) is the 0x5555/0xAAAA service sequence; we accept it
        // without modeling the bite, so just store every register.
        let i = self.idx(offset);
        self.regs[i] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wdog_reset_values() {
        let mut w = Wdog::new(Kind::Wdog);
        assert_eq!(w.read(0x00), 0x0030); // WCR
        w.write(0x00, 0x0000); // clear WDE etc.
        assert_eq!(w.read(0x00), 0x0000);
    }

    #[test]
    fn rtwdog_reset_values_and_service() {
        let mut w = Wdog::new(Kind::RtWdog);
        assert_eq!(w.read(0x00), 0x0000_2980); // CS
        assert_eq!(w.read(0x08), 0x0000_0400); // TOVAL
        w.write(0x00, 0x0000_0000); // disable after unlock
        assert_eq!(w.read(0x00), 0);
    }
}

// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! CCM — Clock Controller Module (MIMXRT1062.h `CCM_Type`; RM §14).
//!
//! Boot code programs the clock tree then spins on handshake/status bits.
//! We model the block as stored-readback (every CCGR gate and divider reads
//! back what firmware wrote, so `CLOCK_EnableClock` / `CLOCK_SetDiv` are
//! observable) with two behavioural fixups the boot path depends on:
//!   * CDHIPR (0x48) reads 0 — no clock-divider handshake is ever busy, so
//!     `CLOCK_SetDiv` spin-loops terminate immediately.
//!   * CCGR0..6 come up all-on (0x_FFFF_FFFF) at reset like the hardware.
//!
//! Register offsets: CCR 0x00, CSR 0x08, CACRR 0x10, CBCDR 0x14, CBCMR 0x18,
//! CSCMR1 0x1C, CDCDR 0x30, CDHIPR 0x48, CLPCR 0x54, CGPR 0x64,
//! CCGR0 0x68 … CCGR6 0x80, CMEOR 0x88.

pub struct Ccm {
    regs: Box<[u32; 0x40]>, // 256-byte register file (64 words)
}

impl Ccm {
    pub fn new() -> Self {
        let mut regs = Box::new([0u32; 0x40]);
        // CCGR0..CCGR6 (0x68..0x80): all clock gates on at reset.
        for off in (0x68..=0x80).step_by(4) {
            regs[off >> 2] = 0xFFFF_FFFF;
        }
        // CSR (0x08): Cn oscillator ready-ish; leave 0 (spin-loops read
        // CCM_ANALOG_MISC0 for XTAL ready, not CCM.CSR, on RT1060).
        Ccm { regs }
    }

    #[inline]
    fn idx(offset: u32) -> usize {
        (offset >> 2) as usize & 0x3F
    }

    /// Side-effect-free register snapshot (for the clock-tree computation).
    #[inline]
    pub fn reg(&self, offset: u32) -> u32 {
        self.regs[Self::idx(offset)]
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x48 => 0, // CDHIPR: no handshake ever busy
            _ => self.regs[Self::idx(offset)],
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        // CDHIPR is read-only; everything else is stored-readback.
        if offset != 0x48 {
            self.regs[Self::idx(offset)] = value;
        }
    }
}

impl Default for Ccm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ccgr_gates_reset_on_and_readback() {
        let mut c = Ccm::new();
        assert_eq!(c.read(0x68), 0xFFFF_FFFF, "CCGR0 on at reset");
        c.write(0x68, 0x0000_00C0);
        assert_eq!(c.read(0x68), 0x0000_00C0);
    }

    #[test]
    fn cdhipr_reads_not_busy() {
        let mut c = Ccm::new();
        c.write(0x48, 0xFFFF_FFFF); // ignored (RO)
        assert_eq!(c.read(0x48), 0, "handshake never busy");
    }
}

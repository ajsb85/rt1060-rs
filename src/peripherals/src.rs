// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! SRC — System Reset Controller (MIMXRT1062.h `SRC_Type`; RM §36).
//!
//! Boot code reads SRSR (the reset-reason latch) and the boot-mode registers
//! (SBMR1/SBMR2) to decide the boot source. We seed SRSR with the power-on
//! reset cause and expose the boot fuse image; a `SCR.SW_*_RST` write is
//! reported to the SoC so it can perform the reset. Everything else is
//! stored-readback.
//!
//! Register offsets: SCR 0x00, SBMR1 0x04, SRSR 0x08, SBMR2 0x1C, GPR1 0x20
//! … GPR10 0x44.

/// SRSR reset-cause bits (RM §36.3.3). Bit 0 = IPP (power-on) reset.
pub const SRSR_POR: u32 = 1 << 0;
pub const SRSR_WDOG: u32 = 1 << 4;
pub const SRSR_WDOG3: u32 = 1 << 6;

pub struct Src {
    scr: u32,
    srsr: u32,
    sbmr1: u32,
    sbmr2: u32,
    gpr: [u32; 10],
    /// Latched when firmware requests a software reset via SCR.
    pub reset_requested: bool,
}

impl Src {
    pub fn new() -> Self {
        Self {
            scr: 0,
            srsr: SRSR_POR,
            // SBMR2: BMOD = internal boot, boot from FlexSPI NOR. The exact
            // fuse image is board-specific; 0 satisfies the SDK's checks.
            sbmr1: 0,
            sbmr2: 0,
            gpr: [0; 10],
            reset_requested: false,
        }
    }

    /// Overwrite the reset cause reported in SRSR (used by the SoC on reset).
    pub fn set_reset_cause(&mut self, cause: u32) {
        self.srsr = cause;
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => self.scr,
            0x04 => self.sbmr1,
            0x08 => self.srsr,
            0x1C => self.sbmr2,
            0x20..=0x44 => self.gpr[((offset - 0x20) >> 2) as usize],
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.scr = value;
                // SCR.MASK_WDOG_RST etc. are config; the low reset-request
                // bits self-clear — report a global reset on any of them.
                if value & 0x1 != 0 {
                    self.reset_requested = true;
                }
            }
            0x08 => self.srsr &= !value, // W1C
            0x20..=0x44 => self.gpr[((offset - 0x20) >> 2) as usize] = value,
            _ => {}
        }
    }
}

impl Default for Src {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srsr_reports_por_and_is_w1c() {
        let mut s = Src::new();
        assert_eq!(s.read(0x08), SRSR_POR);
        s.write(0x08, SRSR_POR); // clear it
        assert_eq!(s.read(0x08), 0);
    }

    #[test]
    fn gpr_roundtrip() {
        let mut s = Src::new();
        s.write(0x20, 0xABCD_1234); // GPR1: bootloader<->app mailbox
        assert_eq!(s.read(0x20), 0xABCD_1234);
    }
}

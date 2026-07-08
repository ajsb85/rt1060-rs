// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! SEMC — Smart External Memory Controller (MIMXRT1062.h `SEMC_Type`;
//! RM §26). The SwiftIO Micro attaches 32 MB of SDRAM through it at
//! `0x8000_0000`.
//!
//! We keep the SDRAM itself as pre-mapped RAM in the bus, so the controller
//! only needs to satisfy the three spin-loops `fsl_semc.c` runs during
//! `SEMC_Init` / `SEMC_ConfigureSDRAM`:
//!
//! - `MCR.SWRST` (0x00 bit 0): software reset; hardware self-clears, so we
//!   drop the bit and it reads back 0 immediately (`SEMC_Init`).
//! - `INTR.IPCMDDONE` (0x14 bit 0): set after every IP command (a write to
//!   `IPCMD` 0xB0 with `KEY = 0xA55A`), W1C; `IPCMDERR` (bit 1) stays 0
//!   (`SEMC_IsIPCommandDone`, called per MODE/precharge/refresh command).
//! - `STS0.IDLE` (0xE0 bit 0): reads 1 (`SEMC_Deinit`).
//!
//! Everything else is stored-readback. Modeling wait states / real command
//! decode is a later ROADMAP item.

const MCR: u32 = 0x00;
const INTR: u32 = 0x14;
const IPCMD: u32 = 0xB0;
const STS0: u32 = 0xE0;

const MCR_SWRST: u32 = 0x1;
const INTR_IPCMDDONE: u32 = 0x1;
const IPCMD_KEY: u32 = 0xA55A_0000; // SEMC_IPCMD_KEY(0xA55A) in [31:16]

pub struct Semc {
    regs: Box<[u32; 4096]>,
    intr: u32,
}

impl Semc {
    pub fn new() -> Self {
        Self {
            regs: Box::new([0; 4096]),
            intr: 0,
        }
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            MCR => self.regs[0] & !MCR_SWRST, // SWRST self-clears
            INTR => self.intr,
            STS0 => 0x1, // IDLE
            _ => self.regs[(offset >> 2) as usize & 4095],
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            MCR => self.regs[0] = value & !MCR_SWRST,
            INTR => self.intr &= !value, // W1C
            IPCMD => {
                self.regs[(IPCMD >> 2) as usize] = value;
                // A valid keyed IP command completes immediately, no error.
                if value & 0xFFFF_0000 == IPCMD_KEY {
                    self.intr |= INTR_IPCMDDONE;
                }
            }
            _ => self.regs[(offset >> 2) as usize & 4095] = value,
        }
    }
}

impl Default for Semc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swrst_self_clears() {
        let mut s = Semc::new();
        s.write(MCR, MCR_SWRST | 0x2); // SWRST + MDIS
        assert_eq!(s.read(MCR) & MCR_SWRST, 0, "SWRST reads back clear");
        assert_ne!(s.read(MCR) & 0x2, 0, "other MCR bits stick");
    }

    #[test]
    fn ip_command_completes() {
        let mut s = Semc::new();
        assert_eq!(s.read(INTR) & INTR_IPCMDDONE, 0);
        s.write(IPCMD, IPCMD_KEY | 0x8); // keyed MODESET command
        assert_ne!(s.read(INTR) & INTR_IPCMDDONE, 0, "IPCMDDONE set");
        assert_eq!(s.read(INTR) & 0x2, 0, "IPCMDERR clear");
        s.write(INTR, INTR_IPCMDDONE); // W1C
        assert_eq!(s.read(INTR) & INTR_IPCMDDONE, 0);
    }

    #[test]
    fn sts0_idle_high() {
        let mut s = Semc::new();
        assert_ne!(s.read(STS0) & 0x1, 0);
    }
}

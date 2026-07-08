// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! FlexSPI — Flexible SPI controller (MIMXRT1062.h `FLEXSPI_Type`; RM §27).
//! FLEXSPI (0x402A_8000, IRQ 108) drives the boot NOR flash; FLEXSPI2
//! (0x402A_4000, IRQ 107) a second port.
//!
//! **XIP reads already work** — the flash AMBA window (`0x6000_0000`) is a
//! backed memory region, so instruction/data fetches resolve directly without
//! the controller. This models the controller *registers* so firmware that
//! reconfigures FlexSPI (LUT, clocks, IP commands) runs without hanging:
//!
//! - `MCR0.SWRESET` self-clears (the `while(MCR0 & SWRESET)` reset spin
//!   terminates);
//! - `STS0` reports `SEQIDLE | ARBIDLE` (the controller is always idle);
//! - `INTR.IPCMDDONE` sets when an IP command is triggered (`IPCMD.TRG`), and
//!   `INTR` is write-1-clear;
//! - the LUT and configuration registers are stored-readback.
//!
//! The IP command engine returns erased data (`0xFFFF_FFFF`) from the RX FIFO;
//! moving real flash bytes through IP read/program (which needs a bus handle
//! like the eDMA / MSC) is a ROADMAP item.
//!
//! Register offsets: MCR0 0x00, INTEN 0x10, INTR 0x14, LUTKEY 0x18,
//! IPCR0 0xA0, IPCMD 0xB0, STS0 0xE0, IPRXFDR 0x100.

const MCR0: u32 = 0x00;
const INTEN: u32 = 0x10;
const INTR: u32 = 0x14;
const IPCMD: u32 = 0xB0;
const STS0: u32 = 0xE0;

const MCR0_SWRESET: u32 = 1 << 0;
const INTR_IPCMDDONE: u32 = 1 << 0;
const INTR_IPRXWA: u32 = 1 << 5; // RX FIFO watermark available
const STS0_SEQIDLE: u32 = 1 << 0;
const STS0_ARBIDLE: u32 = 1 << 1;
const IPCMD_TRG: u32 = 1 << 0;

pub struct FlexSpi {
    pub index: u8,
    regs: [u32; 0x100], // 1 KiB register file
    intr: u32,
}

impl FlexSpi {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            regs: [0; 0x100],
            intr: 0,
        }
    }

    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            MCR0 => self.regs[0] & !MCR0_SWRESET, // SWRESET self-cleared
            INTR => self.intr,
            STS0 => STS0_SEQIDLE | STS0_ARBIDLE, // always idle
            0x100..=0x17C => 0xFFFF_FFFF,        // IP RX FIFO: erased flash
            _ => self.regs[(off >> 2) as usize & 0xFF],
        }
    }

    pub fn write(&mut self, off: u32, value: u32) {
        match off {
            MCR0 => self.regs[0] = value & !MCR0_SWRESET,
            INTR => self.intr &= !value, // W1C
            IPCMD => {
                if value & IPCMD_TRG != 0 {
                    self.intr |= INTR_IPCMDDONE | INTR_IPRXWA;
                }
            }
            _ => self.regs[(off >> 2) as usize & 0xFF] = value,
        }
    }

    pub fn irq_pending(&self) -> bool {
        self.intr & self.regs[(INTEN >> 2) as usize] != 0
    }
}

impl Default for FlexSpi {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swreset_self_clears_and_controller_reads_idle() {
        let mut f = FlexSpi::new(1);
        f.write(MCR0, MCR0_SWRESET | 0x2);
        assert_eq!(f.read(MCR0) & MCR0_SWRESET, 0, "SWRESET self-cleared");
        assert_ne!(f.read(MCR0) & 0x2, 0, "other MCR0 bits stick");
        assert_eq!(
            f.read(STS0),
            STS0_SEQIDLE | STS0_ARBIDLE,
            "controller reports idle"
        );
    }

    #[test]
    fn ip_command_completes_and_gates_irq() {
        let mut f = FlexSpi::new(1);
        f.write(INTEN, INTR_IPCMDDONE);
        assert!(!f.irq_pending());
        f.write(IPCMD, IPCMD_TRG); // trigger an IP command
        assert_ne!(f.read(INTR) & INTR_IPCMDDONE, 0);
        assert!(f.irq_pending());
        f.write(INTR, INTR_IPCMDDONE); // W1C
        assert!(!f.irq_pending());
    }

    #[test]
    fn ip_rx_fifo_reads_erased() {
        let mut f = FlexSpi::new(1);
        assert_eq!(f.read(0x100), 0xFFFF_FFFF);
    }
}

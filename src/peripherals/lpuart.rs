// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! LPUART — Low-Power UART (MIMXRT1062.h `LPUART_Type`; RM §51).
//!
//! LPUART1 is the SwiftIO console: Zephyr's `printf`/newlib retarget and the
//! USB-serial bridge used by `mm download` both route through it, so this is
//! the first peripheral that makes a boot observable. The model transmits
//! instantly (host-side string sink) and exposes an RX FIFO the host can
//! push into; STATUS flags and the TX/RX/TC interrupt sources are modeled so
//! interrupt-driven drivers behave.
//!
//! Register offsets (MIMXRT1062.h): VERID 0x00, PARAM 0x04, GLOBAL 0x08,
//! PINCFG 0x0C, BAUD 0x10, STAT 0x14, CTRL 0x18, DATA 0x1C, MATCH 0x20,
//! MODIR 0x24, FIFO 0x28, WATER 0x2C.

use std::collections::VecDeque;

// STAT (0x14) flags.
const STAT_TDRE: u32 = 1 << 23; // transmit data register empty
const STAT_TC: u32 = 1 << 22; // transmission complete
const STAT_RDRF: u32 = 1 << 21; // receive data register full
const STAT_IDLE: u32 = 1 << 20;

// CTRL (0x18) fields.
const CTRL_TIE: u32 = 1 << 23; // TDRE interrupt enable
const CTRL_TCIE: u32 = 1 << 22; // TC interrupt enable
const CTRL_RIE: u32 = 1 << 21; // RDRF interrupt enable
const CTRL_TE: u32 = 1 << 19; // transmitter enable
#[allow(dead_code)] // RE is honoured by real drivers; RX push bypasses it here
const CTRL_RE: u32 = 1 << 18; // receiver enable

pub struct LpUart {
    /// Instance number (1..8) for trace labels.
    pub index: u8,
    ctrl: u32,
    baud: u32,
    /// Sticky STAT bits the driver clears by W1C / read sequences.
    stat_sticky: u32,
    rx: VecDeque<u8>,
    /// Everything the firmware has transmitted (the "serial cable").
    out: Vec<u8>,
}

impl LpUart {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            ctrl: 0,
            baud: 0x0F00_0004, // reset value (RM §51: OSR=15, SBR=4)
            stat_sticky: 0,
            rx: VecDeque::new(),
            out: Vec::new(),
        }
    }

    /// Host pushes a received byte into the RX FIFO.
    pub fn rx_push(&mut self, byte: u8) {
        self.rx.push_back(byte);
    }

    /// Drain everything transmitted since the last call, as bytes.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// Drain everything transmitted since the last call, as a UTF-8 string
    /// (lossy — firmware may emit control bytes).
    pub fn take_output_string(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.out)).into_owned()
    }

    /// Current STAT value (transmitter is always ready — instant TX).
    fn stat(&self) -> u32 {
        let mut s = self.stat_sticky;
        if self.ctrl & CTRL_TE != 0 {
            s |= STAT_TDRE | STAT_TC;
        }
        if !self.rx.is_empty() {
            s |= STAT_RDRF;
        }
        s
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => 0x0401_0003, // VERID: LPUART v4 (RM §51.4.1)
            0x04 => 0x0000_0202, // PARAM: TX/RX FIFO depth codes (4-entry)
            0x08 => 0,           // GLOBAL
            0x10 => self.baud,
            0x14 => self.stat(),
            0x18 => self.ctrl,
            0x1C => {
                // DATA: pop a received byte; IDLE/empty reads return 0.
                self.rx.pop_front().map_or(0, u32::from)
            }
            0x28 => 0x00C0_0011, // FIFO: TX/RX FIFO present, size fields
            0x2C => 0,           // WATER
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x08 => {
                // GLOBAL.RST (bit 1): software reset clears state.
                if value & 0x2 != 0 {
                    self.ctrl = 0;
                    self.rx.clear();
                    self.stat_sticky = 0;
                }
            }
            0x10 => self.baud = value,
            0x14 => {
                // STAT is largely W1C for the error/idle flags; TDRE/TC/RDRF
                // are read-only status. Clear only the sticky lane.
                self.stat_sticky &= !(value & STAT_IDLE);
            }
            0x18 => self.ctrl = value,
            0x1C => {
                // DATA write: transmit the low byte immediately.
                if self.ctrl & CTRL_TE != 0 {
                    self.out.push(value as u8);
                }
            }
            _ => {}
        }
    }

    /// DMA transmit request: `BAUD.TDMAE` set and the transmitter ready.
    pub fn dma_tx_request(&self) -> bool {
        self.baud & (1 << 23) != 0 && self.ctrl & CTRL_TE != 0
    }

    /// DMA receive request: `BAUD.RDMAE` set and a byte waiting.
    pub fn dma_rx_request(&self) -> bool {
        self.baud & (1 << 21) != 0 && !self.rx.is_empty()
    }

    /// Level-sensitive interrupt request: TDRE/TC (always ready when TE) or
    /// RDRF, each gated by its CTRL enable.
    pub fn irq_pending(&self) -> bool {
        let stat = self.stat();
        (self.ctrl & CTRL_TIE != 0 && stat & STAT_TDRE != 0)
            || (self.ctrl & CTRL_TCIE != 0 && stat & STAT_TC != 0)
            || (self.ctrl & CTRL_RIE != 0 && stat & STAT_RDRF != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_reaches_host_and_sets_flags() {
        let mut u = LpUart::new(1);
        u.write(0x18, CTRL_TE | CTRL_RE); // CTRL: TE|RE
        u.write(0x1C, u32::from(b'h'));
        u.write(0x1C, u32::from(b'i'));
        assert_eq!(u.take_output_string(), "hi");
        assert_ne!(u.read(0x14) & STAT_TC, 0, "TC set after tx");
    }

    #[test]
    fn rx_fifo_and_rdrf() {
        let mut u = LpUart::new(1);
        u.write(0x18, CTRL_RE | CTRL_RIE);
        assert_eq!(u.read(0x14) & STAT_RDRF, 0);
        u.rx_push(b'z');
        assert_ne!(u.read(0x14) & STAT_RDRF, 0);
        assert!(u.irq_pending(), "RIE + RDRF raises IRQ");
        assert_eq!(u.read(0x1C), u32::from(b'z')); // DATA pop
        assert_eq!(u.read(0x14) & STAT_RDRF, 0, "RDRF clears when drained");
    }

    #[test]
    fn tx_disabled_swallows_data() {
        let mut u = LpUart::new(1);
        u.write(0x1C, u32::from(b'x')); // TE not set
        assert_eq!(u.take_output().len(), 0);
    }
}

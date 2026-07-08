// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! FlexCAN — CAN controller (MIMXRT1062.h `CAN_Type`; RM §44). CAN1..3
//! (IRQ 36/37/154). The SwiftIO Micro exposes one CAN bus.
//!
//! Models the message-buffer engine and the MCR freeze/reset handshake the
//! init spins on:
//!
//! - `MCR.SOFTRST` self-clears; `MCR.FRZACK`/`NOTRDY` track freeze/disable so
//!   `while(!(MCR & FRZACK))` and `while(MCR & NOTRDY)` terminate.
//! - Writing a message buffer's CS word with `CODE = 0xC` (TX DATA) transmits
//!   the frame (to the `tx` log, and — with `CTRL1.LPB` loopback — into a
//!   matching RX buffer); the buffer's interrupt flag is set in `IFLAG1`.
//! - [`FlexCan::rx_push`] delivers a received frame into the first empty RX
//!   buffer (`CODE = 0x4`), sets it full, and flags `IFLAG1`.
//! - `IFLAG1` is write-1-clear and, gated by `IMASK1`, raises the IRQ.
//!
//! Message buffers start at offset 0x80, 16 bytes each: CS 0x00, ID 0x04,
//! WORD0 0x08, WORD1 0x0C. CAN payload bytes are big-endian in WORD0/WORD1.

const MCR: u32 = 0x00;
const CTRL1: u32 = 0x04;
const IMASK1: u32 = 0x28;
const IFLAG1: u32 = 0x30;
const MB_BASE: u32 = 0x80;
const MB_STRIDE: u32 = 0x10;
const NUM_MB: u32 = 64;

const MCR_MDIS: u32 = 1 << 31;
const MCR_FRZ: u32 = 1 << 30;
const MCR_HALT: u32 = 1 << 28;
const MCR_NOTRDY: u32 = 1 << 27;
const MCR_SOFTRST: u32 = 1 << 25;
const MCR_FRZACK: u32 = 1 << 24;
const CTRL1_LPB: u32 = 1 << 12; // loopback

// MB CS CODE values (bits [27:24]).
const CODE_RX_EMPTY: u32 = 0x4;
const CODE_RX_FULL: u32 = 0x2;
const CODE_TX_DATA: u32 = 0xC;
const CODE_TX_INACTIVE: u32 = 0x8;

/// A CAN frame on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u32,
    pub extended: bool,
    pub rtr: bool,
    pub data: Vec<u8>,
}

pub struct FlexCan {
    pub index: u8,
    regs: Box<[u32; 1024]>, // 4 KiB register + MB window
    iflag1: u32,
    /// Frames transmitted onto the bus (the "CAN cable").
    tx: Vec<CanFrame>,
}

impl FlexCan {
    pub fn new(index: u8) -> Self {
        let mut regs = Box::new([0u32; 1024]);
        // MCR reset value (RM §44.7.1): MDIS|FRZ|HALT set, MAXMB=0x0F.
        regs[(MCR >> 2) as usize] = MCR_MDIS | MCR_FRZ | MCR_HALT | 0x000F;
        Self {
            index,
            regs,
            iflag1: 0,
            tx: Vec::new(),
        }
    }

    /// Drain the frames transmitted since the last call.
    pub fn take_tx(&mut self) -> Vec<CanFrame> {
        std::mem::take(&mut self.tx)
    }

    #[inline]
    fn r(&self, off: u32) -> u32 {
        self.regs[(off >> 2) as usize & 1023]
    }
    #[inline]
    fn w(&mut self, off: u32, v: u32) {
        self.regs[(off >> 2) as usize & 1023] = v;
    }

    fn mcr(&self) -> u32 {
        let m = self.r(MCR) & !MCR_SOFTRST; // SOFTRST self-cleared
        let frozen = m & MCR_MDIS != 0 || (m & MCR_FRZ != 0 && m & MCR_HALT != 0);
        let mut v = m & !(MCR_FRZACK | MCR_NOTRDY);
        if frozen {
            v |= MCR_FRZACK | MCR_NOTRDY;
        }
        v
    }

    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            MCR => self.mcr(),
            IFLAG1 => self.iflag1,
            _ => self.r(off),
        }
    }

    pub fn write(&mut self, off: u32, value: u32) {
        match off {
            MCR => self.w(MCR, value & !MCR_SOFTRST),
            IFLAG1 => self.iflag1 &= !value, // W1C
            _ if is_cs_word(off) => {
                self.w(off, value);
                if (value >> 24) & 0xF == CODE_TX_DATA {
                    self.transmit(off, value);
                }
            }
            _ => self.w(off, value),
        }
    }

    fn transmit(&mut self, cs_off: u32, cs: u32) {
        let mb = (cs_off - MB_BASE) / MB_STRIDE;
        let extended = cs & (1 << 21) != 0; // IDE
        let id_reg = self.r(cs_off + 4);
        let id = if extended {
            id_reg & 0x1FFF_FFFF
        } else {
            (id_reg >> 18) & 0x7FF
        };
        let dlc = ((cs >> 16) & 0xF) as usize;
        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&self.r(cs_off + 8).to_be_bytes());
        data.extend_from_slice(&self.r(cs_off + 0xC).to_be_bytes());
        data.truncate(dlc.min(8));
        let frame = CanFrame {
            id,
            extended,
            rtr: cs & (1 << 20) != 0,
            data,
        };
        // The buffer becomes TX-inactive once sent.
        self.w(cs_off, (cs & !0x0F00_0000) | (CODE_TX_INACTIVE << 24));
        self.iflag1 |= 1 << mb;
        if self.r(CTRL1) & CTRL1_LPB != 0 {
            self.deliver(&frame); // loopback into an RX buffer
        }
        self.tx.push(frame);
    }

    /// Deliver a received frame into the first empty RX buffer.
    pub fn rx_push(&mut self, frame: &CanFrame) -> bool {
        self.deliver(frame)
    }

    fn deliver(&mut self, frame: &CanFrame) -> bool {
        for mb in 0..NUM_MB {
            let off = MB_BASE + mb * MB_STRIDE;
            let cs = self.r(off);
            if (cs >> 24) & 0xF != CODE_RX_EMPTY {
                continue;
            }
            let id_field = if frame.extended {
                frame.id & 0x1FFF_FFFF
            } else {
                (frame.id & 0x7FF) << 18
            };
            let dlc = frame.data.len().min(8);
            let mut new_cs = (CODE_RX_FULL << 24) | ((dlc as u32) << 16);
            if frame.extended {
                new_cs |= (1 << 21) | (1 << 22); // IDE | SRR
            }
            if frame.rtr {
                new_cs |= 1 << 20;
            }
            self.w(off, new_cs);
            self.w(off + 4, id_field);
            let mut w = [0u8; 8];
            w[..dlc].copy_from_slice(&frame.data[..dlc]);
            self.w(off + 8, u32::from_be_bytes(w[0..4].try_into().unwrap()));
            self.w(off + 0xC, u32::from_be_bytes(w[4..8].try_into().unwrap()));
            self.iflag1 |= 1 << mb;
            return true;
        }
        false
    }

    pub fn irq_pending(&self) -> bool {
        self.iflag1 & self.r(IMASK1) != 0
    }
}

impl Default for FlexCan {
    fn default() -> Self {
        Self::new(1)
    }
}

#[inline]
fn is_cs_word(off: u32) -> bool {
    (MB_BASE..MB_BASE + NUM_MB * MB_STRIDE).contains(&off)
        && (off - MB_BASE).is_multiple_of(MB_STRIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(n: u32) -> u32 {
        MB_BASE + n * MB_STRIDE
    }

    #[test]
    fn mcr_freeze_handshake() {
        let mut c = FlexCan::new(1);
        // Reset: frozen (MDIS|FRZ|HALT) → FRZACK + NOTRDY read set.
        assert_ne!(c.read(MCR) & MCR_FRZACK, 0);
        // Leave freeze: clear MDIS/FRZ/HALT.
        c.write(MCR, 0x0000_000F);
        assert_eq!(c.read(MCR) & MCR_NOTRDY, 0, "running: not-ready clears");
        assert_eq!(c.read(MCR) & MCR_FRZACK, 0);
        // SOFTRST self-clears.
        c.write(MCR, MCR_SOFTRST);
        assert_eq!(c.read(MCR) & MCR_SOFTRST, 0);
    }

    #[test]
    fn transmit_frame_from_message_buffer() {
        let mut c = FlexCan::new(1);
        c.write(mb(0) + 4, 0x123 << 18); // ID = 0x123 (standard)
        c.write(mb(0) + 8, 0xDEAD_BEEF); // WORD0
        c.write(mb(0) + 0xC, 0xCAFE_0000); // WORD1
        // CS: CODE=TX_DATA, DLC=6.
        c.write(mb(0), (CODE_TX_DATA << 24) | (6 << 16));
        let frames = c.take_tx();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].id, 0x123);
        assert_eq!(frames[0].data, vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE]);
        assert_ne!(c.read(IFLAG1) & 1, 0, "MB0 interrupt flagged");
    }

    #[test]
    fn rx_push_fills_an_empty_buffer() {
        let mut c = FlexCan::new(1);
        c.write(mb(5), CODE_RX_EMPTY << 24); // MB5 = RX EMPTY
        c.write(IMASK1, 1 << 5); // enable MB5 interrupt
        let f = CanFrame {
            id: 0x321,
            extended: false,
            rtr: false,
            data: vec![1, 2, 3, 4],
        };
        assert!(c.rx_push(&f));
        assert_eq!((c.read(mb(5)) >> 24) & 0xF, CODE_RX_FULL, "MB5 now full");
        assert_eq!(c.read(mb(5) + 4) >> 18, 0x321);
        assert_eq!(c.read(mb(5) + 8), 0x0102_0304);
        assert!(c.irq_pending());
        c.write(IFLAG1, 1 << 5); // W1C
        assert!(!c.irq_pending());
    }

    #[test]
    fn loopback_delivers_tx_to_rx() {
        let mut c = FlexCan::new(1);
        c.write(CTRL1, CTRL1_LPB); // loopback mode
        c.write(mb(1), CODE_RX_EMPTY << 24); // RX buffer ready
        // TX from MB0.
        c.write(mb(0) + 4, 0x100 << 18);
        c.write(mb(0) + 8, 0x1122_3344);
        c.write(mb(0), (CODE_TX_DATA << 24) | (4 << 16));
        assert_eq!(
            (c.read(mb(1)) >> 24) & 0xF,
            CODE_RX_FULL,
            "loopback filled MB1"
        );
        assert_eq!(c.read(mb(1) + 8), 0x1122_3344);
    }
}

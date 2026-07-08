// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! LPSPI — Low-Power SPI master (MIMXRT1062.h `LPSPI_Type`; RM §48).
//!
//! Full-duplex master: every word firmware writes to `TDR` is shifted out and
//! a word is shifted in from the attached [`SpiDevice`] (or looped back when
//! no device is attached), landing in the RX FIFO for `RDR` to pop. The frame
//! size comes from `TCR.FRAMESZ`. DMA, slave mode, and CS timing are ROADMAP
//! items.
//!
//! Register map (offsets): CR 0x10, SR 0x14, IER 0x18, CFGR1 0x24, FSR 0x5C,
//! TCR 0x60, TDR 0x64, RSR 0x70, RDR 0x74.

use std::collections::VecDeque;

const CR_MEN: u32 = 1 << 0; // module enable
const CR_RST: u32 = 1 << 1; // software reset
const CR_RTF: u32 = 1 << 8; // reset TX FIFO
const CR_RRF: u32 = 1 << 9; // reset RX FIFO

const SR_TDF: u32 = 1 << 0; // TX data flag (FIFO below watermark)
const SR_RDF: u32 = 1 << 1; // RX data flag (FIFO above watermark)
const SR_WCF: u32 = 1 << 8; // word complete
const SR_FCF: u32 = 1 << 9; // frame complete
const SR_TCF: u32 = 1 << 10; // transfer complete
// SR_MBF (module busy, bit 24) is never observably set: transfers complete
// instantly, so the module is never caught mid-transfer.

const RSR_RXEMPTY: u32 = 1 << 1;
const TCR_FRAMESZ: u32 = 0xFFF; // frame size minus one, in bits

/// A device on the far side of an SPI bus. `transfer` returns the MISO word
/// shifted in while `mosi` shifts out. Off the CPU execution path.
pub trait SpiDevice {
    fn transfer(&mut self, mosi: u32) -> u32;
}

pub struct LpSpi {
    pub index: u8,
    cr: u32,
    ier: u32,
    der: u32,
    tcr: u32,
    /// Sticky status the driver clears via W1C (WCF/FCF/TCF).
    sr_sticky: u32,
    rx: VecDeque<u32>,
    device: Option<Box<dyn SpiDevice + Send>>,
}

impl LpSpi {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            cr: 0,
            ier: 0,
            der: 0,
            tcr: 0,
            sr_sticky: 0,
            rx: VecDeque::new(),
            device: None,
        }
    }

    /// Attach the device selected on this bus (single-target for now).
    pub fn attach(&mut self, dev: Box<dyn SpiDevice + Send>) {
        self.device = Some(dev);
    }

    fn sr(&self) -> u32 {
        let mut s = self.sr_sticky;
        if self.cr & CR_MEN != 0 {
            s |= SR_TDF; // TX FIFO always drained (instant shift)
        }
        if !self.rx.is_empty() {
            s |= SR_RDF;
        }
        s
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => 0x0101_0004, // VERID
            0x04 => 0x0002_0202, // PARAM: 4-entry FIFOs, PCS count
            0x10 => self.cr,
            0x14 => self.sr(),
            0x18 => self.ier,
            0x5C => (self.rx.len() as u32) << 16, // FSR: RXCOUNT [18:16]
            0x60 => self.tcr,
            0x70 => {
                if self.rx.is_empty() {
                    RSR_RXEMPTY
                } else {
                    0
                }
            }
            0x74 => self.rx.pop_front().unwrap_or(0), // RDR
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x10 => {
                if value & CR_RST != 0 {
                    self.reset();
                }
                if value & CR_RRF != 0 {
                    self.rx.clear();
                }
                let _ = CR_RTF; // TX FIFO never backs up (instant shift)
                self.cr = value & !(CR_RST | CR_RTF | CR_RRF);
            }
            0x14 => self.sr_sticky &= !(value & (SR_WCF | SR_FCF | SR_TCF)),
            0x18 => self.ier = value,
            0x1C => self.der = value, // DMA enable
            0x60 => self.tcr = value,
            0x64 => self.transmit(value), // TDR
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.cr = 0;
        self.ier = 0;
        self.tcr = 0;
        self.sr_sticky = 0;
        self.rx.clear();
    }

    /// Shift one word out (and a MISO word in), masked to FRAMESZ.
    fn transmit(&mut self, word: u32) {
        if self.cr & CR_MEN == 0 {
            return;
        }
        let bits = (self.tcr & TCR_FRAMESZ) + 1;
        let mask = if bits >= 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        let mosi = word & mask;
        let miso = match self.device.as_mut() {
            Some(d) => d.transfer(mosi) & mask,
            None => mosi, // loopback
        };
        self.rx.push_back(miso);
        self.sr_sticky |= SR_WCF | SR_FCF | SR_TCF;
    }

    pub fn irq_pending(&self) -> bool {
        self.sr() & self.ier != 0
    }

    /// DMA transmit request: `DER.TDDE` set and the module ready to shift.
    pub fn dma_tx_request(&self) -> bool {
        self.der & 0x1 != 0 && self.cr & CR_MEN != 0
    }

    /// DMA receive request: `DER.RDDE` set and a word waiting.
    pub fn dma_rx_request(&self) -> bool {
        self.der & 0x2 != 0 && !self.rx.is_empty()
    }
}

/// A device that replies with a preset word sequence (a sensor streaming
/// samples), ignoring MOSI. Handy as a test/demo target.
pub struct SeqSpiDevice {
    replies: VecDeque<u32>,
    idle: u32,
}

impl SeqSpiDevice {
    pub fn new(replies: impl IntoIterator<Item = u32>, idle: u32) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            idle,
        }
    }
}

impl SpiDevice for SeqSpiDevice {
    fn transfer(&mut self, _mosi: u32) -> u32 {
        self.replies.pop_front().unwrap_or(self.idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_when_no_device() {
        let mut spi = LpSpi::new(1);
        spi.write(0x10, CR_MEN); // CR.MEN
        spi.write(0x60, 7); // TCR.FRAMESZ = 7 → 8-bit frames
        spi.write(0x64, 0xA5); // TDR
        assert_ne!(spi.read(0x14) & SR_RDF, 0, "RX has the looped-back word");
        assert_ne!(spi.read(0x14) & SR_TCF, 0, "transfer complete");
        assert_eq!(spi.read(0x74) & 0xFF, 0xA5, "RDR = MOSI (loopback)");
    }

    #[test]
    fn device_supplies_miso() {
        let mut spi = LpSpi::new(1);
        spi.write(0x10, CR_MEN);
        spi.write(0x60, 7); // 8-bit
        spi.attach(Box::new(SeqSpiDevice::new([0xDE, 0xAD], 0x00)));
        spi.write(0x64, 0x00); // clock out a dummy byte
        spi.write(0x64, 0x00);
        assert_eq!(spi.read(0x74) & 0xFF, 0xDE);
        assert_eq!(spi.read(0x74) & 0xFF, 0xAD);
        // FIFO drained → RSR.RXEMPTY.
        assert_ne!(spi.read(0x70) & RSR_RXEMPTY, 0);
    }

    #[test]
    fn frame_size_masks_word() {
        let mut spi = LpSpi::new(1);
        spi.write(0x10, CR_MEN);
        spi.write(0x60, 15); // 16-bit frames
        spi.write(0x64, 0x1234_ABCD);
        assert_eq!(spi.read(0x74), 0xABCD, "masked to 16 bits");
    }

    #[test]
    fn irq_on_transfer_complete_when_enabled() {
        let mut spi = LpSpi::new(1);
        spi.write(0x10, CR_MEN);
        spi.write(0x18, SR_TCF); // IER: transfer-complete interrupt
        spi.write(0x60, 7);
        spi.write(0x64, 0x11);
        assert!(spi.irq_pending());
        spi.write(0x14, SR_TCF); // W1C clears the transfer-complete flag
        assert!(!spi.irq_pending(), "only TCF was enabled in IER");
    }
}

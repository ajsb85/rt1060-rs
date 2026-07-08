// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! SAI — Synchronous Audio Interface / I²S (MIMXRT1062.h `I2S_Type`; RM §38).
//! SAI1..3 (IRQ 56/57/58). The SwiftIO Micro exposes one I²S bus.
//!
//! Models the transmit/receive FIFO data path: words written to `TDR0` (when
//! `TCSR.TE`) are appended to the transmitted sample stream (the audio out),
//! and words pushed by the host are popped from `RDR0`. `TCSR`/`RCSR` report a
//! FIFO request (`FRF`) whenever the transmitter is ready / the receiver has
//! data, gating the FIFO interrupt (`FRIE`) and — for wiring later — the DMA
//! request (`FRDE`). Bit-clock/frame-sync generation and multi-channel FIFO
//! word counts are ROADMAP items.
//!
//! Register offsets: TCSR 0x08, TDR0 0x20, RCSR 0x88, RDR0 0xA0.

use std::collections::VecDeque;

const TCSR: u32 = 0x08;
const TDR0: u32 = 0x20;
const RCSR: u32 = 0x88;
const RDR0: u32 = 0xA0;

const CSR_TE: u32 = 1 << 31; // transmit/receive enable
const CSR_FRIE: u32 = 1 << 8; // FIFO request interrupt enable
const CSR_FRDE: u32 = 1 << 0; // FIFO request DMA enable
const CSR_FRF: u32 = 1 << 16; // FIFO request flag
const CSR_FWF: u32 = 1 << 17; // FIFO warning flag

pub struct Sai {
    pub index: u8,
    regs: [u32; 64],
    /// Transmitted audio words (the I²S output stream).
    tx: Vec<u32>,
    rx: VecDeque<u32>,
}

impl Sai {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            regs: [0; 64],
            tx: Vec::new(),
            rx: VecDeque::new(),
        }
    }

    /// Drain the audio words transmitted since the last call.
    pub fn take_output(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.tx)
    }

    /// Push a received audio word into the RX FIFO.
    pub fn rx_push(&mut self, word: u32) {
        self.rx.push_back(word);
    }

    #[inline]
    fn r(&self, off: u32) -> u32 {
        self.regs[(off >> 2) as usize & 63]
    }

    fn tcsr(&self) -> u32 {
        let mut v = self.r(TCSR);
        if v & CSR_TE != 0 {
            v |= CSR_FRF | CSR_FWF; // TX FIFO always ready (instant drain)
        }
        v
    }

    fn rcsr(&self) -> u32 {
        let mut v = self.r(RCSR);
        if !self.rx.is_empty() {
            v |= CSR_FRF;
        }
        v
    }

    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            TCSR => self.tcsr(),
            RCSR => self.rcsr(),
            RDR0..=0xAC => self.rx.pop_front().unwrap_or(0),
            _ => self.r(off),
        }
    }

    pub fn write(&mut self, off: u32, value: u32) {
        match off {
            TDR0..=0x2C => {
                if self.r(TCSR) & CSR_TE != 0 {
                    self.tx.push(value);
                }
            }
            _ => self.regs[(off >> 2) as usize & 63] = value,
        }
    }

    pub fn irq_pending(&self) -> bool {
        (self.r(TCSR) & CSR_FRIE != 0 && self.tcsr() & CSR_FRF != 0)
            || (self.r(RCSR) & CSR_FRIE != 0 && self.rcsr() & CSR_FRF != 0)
    }

    /// DMA transmit request: `TCSR.FRDE` set and the transmitter enabled.
    pub fn dma_tx_request(&self) -> bool {
        self.r(TCSR) & CSR_FRDE != 0 && self.r(TCSR) & CSR_TE != 0
    }

    /// DMA receive request: `RCSR.FRDE` set and a word waiting.
    pub fn dma_rx_request(&self) -> bool {
        self.r(RCSR) & CSR_FRDE != 0 && !self.rx.is_empty()
    }
}

impl Default for Sai {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transmit_words_reach_the_output_stream() {
        let mut s = Sai::new(1);
        s.write(TCSR, CSR_TE); // enable transmitter
        s.write(TDR0, 0x1111_2222);
        s.write(TDR0, 0x3333_4444);
        assert_eq!(s.take_output(), vec![0x1111_2222, 0x3333_4444]);
        assert_ne!(s.read(TCSR) & CSR_FRF, 0, "FIFO request while enabled");
    }

    #[test]
    fn disabled_transmitter_drops_data() {
        let mut s = Sai::new(1);
        s.write(TDR0, 0xDEAD); // TE not set
        assert!(s.take_output().is_empty());
    }

    #[test]
    fn receive_fifo_and_interrupt() {
        let mut s = Sai::new(1);
        s.write(RCSR, CSR_TE | CSR_FRIE); // RE + FIFO-request interrupt
        assert!(!s.irq_pending());
        s.rx_push(0xABCD_1234);
        assert_ne!(s.read(RCSR) & CSR_FRF, 0);
        assert!(s.irq_pending());
        assert_eq!(s.read(RDR0), 0xABCD_1234);
        assert!(!s.irq_pending(), "request drops after drain");
    }

    #[test]
    fn dma_requests_gated_by_frde() {
        let mut s = Sai::new(1);
        s.write(TCSR, CSR_TE | CSR_FRDE);
        assert!(s.dma_tx_request());
        s.write(RCSR, CSR_TE | CSR_FRDE);
        assert!(!s.dma_rx_request(), "no rx data yet");
        s.rx_push(1);
        assert!(s.dma_rx_request());
    }
}

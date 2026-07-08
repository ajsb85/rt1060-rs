// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! eDMA — enhanced DMA controller (MIMXRT1062.h `DMA_Type`, base
//! `0x400E_8000`; RM §4). 32 channels, each driven by a Transfer Control
//! Descriptor (TCD) in the register file. The engine moves data *through the
//! system bus* (source/dest may be RAM or a peripheral FIFO register), so —
//! like the mg24-rs LDMA — it is serviced with a bus handle
//! ([`DmaMem`]) via the SoC's swap-out / service / swap-back pattern.
//!
//! Modeled now: software-started transfers (`TCD.CSR.START` or the `SSRT`
//! action register), the minor loop (`NBYTES`, `SSIZE`/`DSIZE`, `SOFF`/`DOFF`)
//! and major loop (`CITER`/`BITER`, `SLAST`/`DLAST`), `INT`/`DONE` and the
//! per-channel IRQ (`DMAn_DMAn+16` = IRQ `n & 15`). Hardware-request triggering
//! via DMAMUX + peripheral request lines is the next ROADMAP step; `service`
//! already accepts a per-channel request slice for it.
//!
//! The TCD holds packed 16-bit fields (CSR and BITER share one word), so the
//! bus must do **width-accurate** writes into this window — the aggregate
//! routes 8/16-bit accesses to [`Edma::write8`]/[`write16`] rather than the
//! replicate-to-word path used for ordinary word registers.

/// Memory as seen by the DMA engine — the system bus.
pub trait DmaMem {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, value: u8);
    fn write16(&mut self, addr: u32, value: u16);
    fn write32(&mut self, addr: u32, value: u32);
}

const NUM_CH: usize = 32;
const TCD_BASE: u32 = 0x1000;
const TCD_STEP: u32 = 0x20;

// TCD field offsets within a channel descriptor.
const T_SADDR: u32 = 0x00;
const T_SOFF: u32 = 0x04;
const T_ATTR: u32 = 0x06;
const T_NBYTES: u32 = 0x08;
const T_SLAST: u32 = 0x0C;
const T_DADDR: u32 = 0x10;
const T_DOFF: u32 = 0x14;
const T_CITER: u32 = 0x16;
const T_DLAST: u32 = 0x18;
const T_CSR: u32 = 0x1C;
const T_BITER: u32 = 0x1E;

// TCD.CSR bits.
const CSR_START: u16 = 1 << 0;
const CSR_INTMAJOR: u16 = 1 << 1;
const CSR_DREQ: u16 = 1 << 3;
const CSR_DONE: u16 = 1 << 7;

pub struct Edma {
    /// 16 KiB register window; only the TCD area (0x1000+) is stored here.
    mem: Box<[u8; 0x4000]>,
    cr: u32,
    /// Enable-request mask (hardware requests), one bit per channel.
    erq: u32,
    /// Interrupt-request mask, one bit per channel.
    int_req: u32,
    err: u32,
}

impl Edma {
    pub fn new() -> Self {
        Self {
            mem: Box::new([0; 0x4000]),
            cr: 0,
            erq: 0,
            int_req: 0,
            err: 0,
        }
    }

    #[inline]
    fn rd16(&self, off: u32) -> u16 {
        let o = off as usize;
        u16::from_le_bytes([self.mem[o], self.mem[o + 1]])
    }
    #[inline]
    fn rd32(&self, off: u32) -> u32 {
        let o = off as usize;
        u32::from_le_bytes(self.mem[o..o + 4].try_into().unwrap())
    }
    #[inline]
    fn wr16(&mut self, off: u32, v: u16) {
        let o = off as usize;
        self.mem[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    fn wr32(&mut self, off: u32, v: u32) {
        let o = off as usize;
        self.mem[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    // --- register access (width-accurate for the TCD window) ---------------

    pub fn read32(&self, off: u32) -> u32 {
        match off {
            0x00 => self.cr,
            0x04 => self.err.count_ones().min(1), // ES: VLD-ish (any error)
            0x0C => self.erq,
            0x24 => self.int_req,
            0x2C => self.err,
            0x34 => 0, // HRS: hardware request status (none tracked yet)
            _ if off >= TCD_BASE => self.rd32(off),
            _ => 0,
        }
    }

    pub fn read16(&self, off: u32) -> u16 {
        if off >= TCD_BASE {
            self.rd16(off)
        } else {
            (self.read32(off & !0x3) >> ((off & 0x2) * 8)) as u16
        }
    }

    pub fn read8(&self, off: u32) -> u8 {
        if off >= TCD_BASE {
            self.mem[off as usize]
        } else {
            (self.read32(off & !0x3) >> ((off & 0x3) * 8)) as u8
        }
    }

    pub fn write32(&mut self, off: u32, value: u32) {
        match off {
            0x00 => self.cr = value,
            0x0C => self.erq = value,
            0x24 => self.int_req &= !value, // W1C
            0x2C => self.err &= !value,
            _ if off >= TCD_BASE => self.wr32(off, value),
            _ => {}
        }
    }

    pub fn write16(&mut self, off: u32, value: u16) {
        if off >= TCD_BASE {
            self.wr16(off, value); // width-accurate: don't clobber the sibling half
        } else {
            self.write32(off & !0x3, u32::from(value) << ((off & 0x2) * 8));
        }
    }

    pub fn write8(&mut self, off: u32, value: u8) {
        match off {
            // Action registers (RM §4.5): write a channel number, or bit 6 =
            // "all channels".
            0x19 => self.set_bits(&mut Field::Erq, value), // SERQ
            0x1A => self.clr_bits(&mut Field::Erq, value), // CERQ
            0x1B => self.set_bits(&mut Field::Erq, value), // SERQ (alt lane)
            0x1D => self.start(value),                     // SSRT: set START
            0x1F => self.clr_bits(&mut Field::Int, value), // CINT
            _ if off >= TCD_BASE => self.mem[off as usize] = value,
            _ => {}
        }
    }

    fn set_bits(&mut self, field: &mut Field, value: u8) {
        let all = value & 0x40 != 0;
        let reg = match field {
            Field::Erq => &mut self.erq,
            Field::Int => &mut self.int_req,
        };
        if all {
            *reg = 0xFFFF_FFFF;
        } else {
            *reg |= 1 << (value & 0x1F);
        }
    }

    fn clr_bits(&mut self, field: &mut Field, value: u8) {
        let all = value & 0x40 != 0;
        let reg = match field {
            Field::Erq => &mut self.erq,
            Field::Int => &mut self.int_req,
        };
        if all {
            *reg = 0;
        } else {
            *reg &= !(1 << (value & 0x1F));
        }
    }

    /// SSRT: set the START bit on a channel (or all).
    fn start(&mut self, value: u8) {
        let set = |e: &mut Edma, ch: usize| {
            let csr = T_CSR + TCD_BASE + ch as u32 * TCD_STEP;
            let v = e.rd16(csr) | CSR_START;
            e.wr16(csr, v);
        };
        if value & 0x40 != 0 {
            for ch in 0..NUM_CH {
                set(self, ch);
            }
        } else {
            set(self, (value & 0x1F) as usize);
        }
    }

    // --- transfer engine ----------------------------------------------------

    /// Service every channel with a pending software START, or an enabled
    /// hardware request (`requests[ch]` && `ERQ[ch]`). Called by the bus with
    /// itself as the DMA's memory view.
    pub fn service(&mut self, bus: &mut impl DmaMem, requests: &[bool]) {
        for ch in 0..NUM_CH {
            let csr_off = TCD_BASE + ch as u32 * TCD_STEP + T_CSR;
            let csr = self.rd16(csr_off);
            let hw = self.erq & (1 << ch) != 0 && requests.get(ch).copied().unwrap_or(false);
            if csr & CSR_START != 0 || hw {
                // Clear START, mark not-done for the run.
                self.wr16(csr_off, csr & !CSR_START & !CSR_DONE);
                self.run_channel(ch, bus);
            }
        }
    }

    fn run_channel(&mut self, ch: usize, bus: &mut impl DmaMem) {
        let base = TCD_BASE + ch as u32 * TCD_STEP;
        let mut saddr = self.rd32(base + T_SADDR);
        let soff = self.rd16(base + T_SOFF) as i16 as i64;
        let attr = self.rd16(base + T_ATTR);
        let nbytes = self.rd32(base + T_NBYTES);
        let mut daddr = self.rd32(base + T_DADDR);
        let doff = self.rd16(base + T_DOFF) as i16 as i64;
        let mut citer = self.rd16(base + T_CITER) & 0x7FFF;
        let biter = self.rd16(base + T_BITER) & 0x7FFF;
        let slast = self.rd32(base + T_SLAST) as i32 as i64;
        let dlast = self.rd32(base + T_DLAST) as i32 as i64;
        let mut csr = self.rd16(base + T_CSR);

        let ssize = 1u32 << ((attr >> 8) & 0x7);
        let dsize = 1u32 << (attr & 0x7);
        let step = ssize.max(dsize).max(1);

        // Minor loop: move NBYTES bytes in max(SSIZE,DSIZE) chunks.
        let mut moved = 0u32;
        while moved < nbytes {
            let val = match ssize {
                1 => u32::from(bus.read8(saddr)),
                2 => u32::from(bus.read16(saddr)),
                _ => bus.read32(saddr),
            };
            match dsize {
                1 => bus.write8(daddr, val as u8),
                2 => bus.write16(daddr, val as u16),
                _ => bus.write32(daddr, val),
            }
            saddr = (saddr as i64 + soff) as u32;
            daddr = (daddr as i64 + doff) as u32;
            moved += step;
        }

        // Major loop bookkeeping.
        citer = citer.wrapping_sub(1);
        if citer == 0 {
            citer = biter;
            saddr = (saddr as i64 + slast) as u32;
            daddr = (daddr as i64 + dlast) as u32;
            if csr & CSR_INTMAJOR != 0 {
                self.int_req |= 1 << ch;
            }
            if csr & CSR_DREQ != 0 {
                self.erq &= !(1 << ch);
            }
            csr |= CSR_DONE;
        }

        self.wr32(base + T_SADDR, saddr);
        self.wr32(base + T_DADDR, daddr);
        self.wr16(base + T_CITER, citer);
        self.wr16(base + T_CSR, csr);
    }

    /// Any channel's interrupt request maps to IRQ `ch & 15` (channel `n` and
    /// `n+16` share a vector). Returns the raw 16-bit line mask.
    pub fn irq_lines16(&self) -> u16 {
        let mut m = 0u16;
        for ch in 0..NUM_CH {
            if self.int_req & (1 << ch) != 0 {
                m |= 1 << (ch & 15);
            }
        }
        m
    }
}

impl Default for Edma {
    fn default() -> Self {
        Self::new()
    }
}

enum Field {
    Erq,
    Int,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat 64 KiB memory for exercising the engine.
    struct FlatMem(Vec<u8>);
    impl DmaMem for FlatMem {
        fn read8(&mut self, a: u32) -> u8 {
            self.0[a as usize]
        }
        fn read16(&mut self, a: u32) -> u16 {
            u16::from_le_bytes([self.0[a as usize], self.0[a as usize + 1]])
        }
        fn read32(&mut self, a: u32) -> u32 {
            u32::from_le_bytes(self.0[a as usize..a as usize + 4].try_into().unwrap())
        }
        fn write8(&mut self, a: u32, v: u8) {
            self.0[a as usize] = v;
        }
        fn write16(&mut self, a: u32, v: u16) {
            self.0[a as usize..a as usize + 2].copy_from_slice(&v.to_le_bytes());
        }
        fn write32(&mut self, a: u32, v: u32) {
            self.0[a as usize..a as usize + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    fn tcd(ch: u32, off: u32) -> u32 {
        TCD_BASE + ch * TCD_STEP + off
    }

    #[test]
    fn packed_16bit_fields_do_not_clobber_siblings() {
        let mut e = Edma::new();
        // BITER (0x1E) and CSR (0x1C) share the word at TCD+0x1C.
        e.write16(tcd(0, T_BITER), 0x1234);
        e.write16(tcd(0, T_CSR), CSR_INTMAJOR);
        assert_eq!(e.read16(tcd(0, T_BITER)), 0x1234, "BITER intact");
        assert_eq!(e.read16(tcd(0, T_CSR)), CSR_INTMAJOR);
    }

    #[test]
    fn mem_to_mem_block_transfer_with_major_interrupt() {
        let mut mem = FlatMem(vec![0; 0x1_0000]);
        for i in 0..16u32 {
            mem.write8(0x100 + i, i as u8);
        }
        let mut e = Edma::new();
        // One channel: 16-byte block, 8-bit transfers, +1 offsets, 1 major.
        e.write32(tcd(0, T_SADDR), 0x100);
        e.write16(tcd(0, T_SOFF), 1);
        e.write16(tcd(0, T_ATTR), 0); // SSIZE=DSIZE=8-bit
        e.write32(tcd(0, T_NBYTES), 16);
        e.write32(tcd(0, T_DADDR), 0x200);
        e.write16(tcd(0, T_DOFF), 1);
        e.write16(tcd(0, T_CITER), 1);
        e.write16(tcd(0, T_BITER), 1);
        e.write16(tcd(0, T_CSR), CSR_INTMAJOR);
        // Software start via SSRT (channel 0).
        e.write8(0x1D, 0);
        e.service(&mut mem, &[]);
        for i in 0..16u32 {
            assert_eq!(mem.read8(0x200 + i), i as u8, "byte {i} copied");
        }
        assert_ne!(e.read32(0x24) & 1, 0, "channel 0 major-interrupt pending");
        assert_ne!(e.irq_lines16() & 1, 0);
        // DONE set, START cleared.
        assert_ne!(e.read16(tcd(0, T_CSR)) & CSR_DONE, 0);
    }

    #[test]
    fn serq_cerq_action_registers() {
        let mut e = Edma::new();
        e.write8(0x1B, 5); // SERQ channel 5
        assert_ne!(e.read32(0x0C) & (1 << 5), 0, "ERQ bit 5 set");
        e.write8(0x1A, 5); // CERQ channel 5
        assert_eq!(e.read32(0x0C) & (1 << 5), 0, "ERQ bit 5 cleared");
        e.write8(0x1B, 0x40); // SERQ all
        assert_eq!(e.read32(0x0C), 0xFFFF_FFFF);
    }

    #[test]
    fn hardware_request_runs_only_when_erq_and_line_set() {
        let mut mem = FlatMem(vec![0; 0x1000]);
        mem.write32(0x10, 0xCAFE_F00D);
        let mut e = Edma::new();
        e.write32(tcd(3, T_SADDR), 0x10);
        e.write16(tcd(3, T_ATTR), (2 << 8) | 2); // 32-bit
        e.write32(tcd(3, T_NBYTES), 4);
        e.write32(tcd(3, T_DADDR), 0x20);
        e.write16(tcd(3, T_CITER), 1);
        e.write16(tcd(3, T_BITER), 1);
        let mut req = [false; 32];
        req[3] = true;
        // No ERQ yet → nothing happens.
        e.service(&mut mem, &req);
        assert_eq!(mem.read32(0x20), 0);
        // Enable the request → transfer runs.
        e.write8(0x1B, 3); // SERQ channel 3
        e.service(&mut mem, &req);
        assert_eq!(mem.read32(0x20), 0xCAFE_F00D);
    }
}

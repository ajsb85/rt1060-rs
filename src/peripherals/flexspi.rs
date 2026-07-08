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
//! The IP command engine moves real flash bytes: an `IPCMD.TRG` classifies the
//! LUT sequence selected by `IPCR1.ISEQID` (does it READ / WRITE / carry a
//! flash address?) and stages a `FlashOp` the bus services against the backed
//! NOR — program, read, or sector erase. Crucially the NXP HAL strobes `IPCMD`
//! *before* streaming the program payload into `IPTXFDR`, so a write sequence
//! is latched at the trigger and finalized once its bytes arrive; a read/erase
//! is staged immediately. This lets the Zephyr NOR / littlefs driver format and
//! mount storage on the emulated flash.
//!
//! Register offsets: MCR0 0x00, INTEN 0x10, INTR 0x14, LUTKEY 0x18,
//! IPCR0 0xA0, IPCMD 0xB0, STS0 0xE0, IPRXFDR 0x100, LUT 0x200.

const MCR0: u32 = 0x00;
const INTEN: u32 = 0x10;
const INTR: u32 = 0x14;
const IPCR0: u32 = 0xA0; // IP control 0: flash device address
const IPCR1: u32 = 0xA4; // IP control 1: IDATSZ [15:0], ISEQID [19:16]
const IPCMD: u32 = 0xB0;
const STS0: u32 = 0xE0;
const IPRXFSTS: u32 = 0xF0; // IP RX FIFO status (FILL count in low byte)
const LUT_BASE_IDX: usize = (0x200 >> 2) as usize; // LUT[0] word index

// LUT instruction opcodes (bits 15:10 of each 16-bit half; fsl_flexspi.h).
const OP_RADDR: u32 = 0x02; // RADDR_SDR — a flash address is part of the sequence
const OP_WRITE: u32 = 0x08; // WRITE_SDR — transmit program data to flash
const OP_READ: u32 = 0x09; // READ_SDR — receive read data from flash

const MCR0_SWRESET: u32 = 1 << 0;
const INTR_IPCMDDONE: u32 = 1 << 0;
const INTR_IPRXWA: u32 = 1 << 5; // RX FIFO watermark available
const INTR_IPTXWE: u32 = 1 << 6; // TX FIFO watermark empty (write available)
const STS0_SEQIDLE: u32 = 1 << 0;
const STS0_ARBIDLE: u32 = 1 << 1;
const IPCMD_TRG: u32 = 1 << 0;

/// A flash operation an IP command triggered, serviced by the bus against the
/// backed FlexSPI NOR region (the FlexSPI peripheral has no memory of its own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlashOp {
    /// Program `data` at flash offset `addr` (NOR program only clears bits).
    Program { addr: u32, data: Vec<u8> },
    /// Read `size` bytes from flash offset `addr` into the RX FIFO.
    Read { addr: u32, size: u32 },
    /// Erase the sector containing `addr` (restore it to `0xFF`).
    Erase { addr: u32 },
}

/// A write sequence triggered by `IPCMD` whose data has not fully arrived yet:
/// the NXP HAL strobes `IPCMD` *before* streaming the payload into `IPTXFDR`,
/// so the program is finalized once `size` bytes have been pushed.
struct WriteOp {
    addr: u32,
    size: u32,
    /// True for a page program (the sequence carries a flash address); false
    /// for a config write like WRSR, whose data has no backing-store effect.
    to_flash: bool,
}

pub struct FlexSpi {
    pub index: u8,
    regs: [u32; 0x100], // 1 KiB register file
    intr: u32,
    /// Bytes written to `IPTXFDR` awaiting a program command.
    tx: Vec<u8>,
    /// Data staged from a flash read, served out of `IPRXFDR`.
    rx: std::collections::VecDeque<u8>,
    /// A flash op the bus must service (set at `IPCMD` strobe).
    pending: Option<FlashOp>,
    /// A write sequence awaiting its `IPTXFDR` payload (see `WriteOp`).
    write_op: Option<WriteOp>,
}

impl FlexSpi {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            regs: [0; 0x100],
            intr: 0,
            tx: Vec::new(),
            rx: std::collections::VecDeque::new(),
            pending: None,
            write_op: None,
        }
    }

    /// Classify the LUT sequence `seqid` selected by `IPCR1.ISEQID`: whether it
    /// reads flash, writes flash, and/or carries a flash address. The IP
    /// command's meaning (read / program / erase / plain command) follows from
    /// these, independent of FIFO timing.
    fn classify_seq(&self, seqid: u32) -> (bool, bool, bool) {
        let base = LUT_BASE_IDX + (seqid as usize & 0xF) * 4;
        let (mut read, mut write, mut raddr) = (false, false, false);
        for k in 0..4 {
            let word = self.regs[base + k];
            for instr in [word & 0xFFFF, word >> 16] {
                match (instr >> 10) & 0x3F {
                    OP_READ => read = true,
                    OP_WRITE => write = true,
                    OP_RADDR => raddr = true,
                    _ => {}
                }
            }
        }
        (read, write, raddr)
    }

    pub fn read(&mut self, off: u32) -> u32 {
        match off {
            MCR0 => self.regs[0] & !MCR0_SWRESET, // SWRESET self-cleared
            // The RX watermark is always available and the TX FIFO always has
            // space, so FLEXSPI_Read/WriteBlocking's polls on INTR.IPRXWA
            // (bit 5) / IPTXWE (bit 6) terminate.
            INTR => self.intr | INTR_IPRXWA | INTR_IPTXWE,
            STS0 => STS0_SEQIDLE | STS0_ARBIDLE, // always idle
            // IPRXFSTS: FILL count (8-byte entries) in the low byte —
            // FLEXSPI_ReadBlocking waits for `remaining <= FILL*8`; report full.
            IPRXFSTS => 0x0000_00FF,
            0x100..=0x17C => {
                // IPRXFDR: pop a staged flash word (little-endian).
                let mut w = [0u8; 4];
                for b in &mut w {
                    *b = self.rx.pop_front().unwrap_or(0xFF);
                }
                u32::from_le_bytes(w)
            }
            _ => self.regs[(off >> 2) as usize & 0xFF],
        }
    }

    pub fn write(&mut self, off: u32, value: u32) {
        match off {
            MCR0 => self.regs[0] = value & !MCR0_SWRESET,
            INTR => self.intr &= !value, // W1C
            0x180..=0x19C => {
                // IPTXFDR: stream program payload. When all bytes of a pending
                // write sequence have arrived, finalize it (a page program
                // reaches the backing store; a config write is consumed).
                self.tx.extend_from_slice(&value.to_le_bytes());
                if let Some(w) = &self.write_op
                    && self.tx.len() >= w.size as usize
                {
                    let w = self.write_op.take().unwrap();
                    let mut data = std::mem::take(&mut self.tx);
                    data.truncate(w.size as usize);
                    if w.to_flash {
                        self.pending = Some(FlashOp::Program { addr: w.addr, data });
                    }
                }
            }
            IPCMD => {
                if value & IPCMD_TRG != 0 {
                    self.intr |= INTR_IPCMDDONE | INTR_IPRXWA | INTR_IPTXWE;
                    let addr = self.regs[(IPCR0 >> 2) as usize];
                    let ipcr1 = self.regs[(IPCR1 >> 2) as usize];
                    let size = ipcr1 & 0xFFFF;
                    let seqid = (ipcr1 >> 16) & 0xF;
                    let (has_read, has_write, has_raddr) = self.classify_seq(seqid);
                    self.tx.clear();
                    self.write_op = None;
                    // The LUT sequence, not FIFO state, decides the operation:
                    // the HAL triggers IPCMD before pushing write data.
                    if has_write {
                        self.write_op = Some(WriteOp {
                            addr,
                            size,
                            to_flash: has_raddr,
                        });
                    } else if has_read {
                        self.pending = Some(FlashOp::Read { addr, size });
                    } else if has_raddr {
                        // A command carrying an address but moving no data is a
                        // sector / block erase.
                        self.pending = Some(FlashOp::Erase { addr });
                    }
                    // else: WREN / WRDI / reset — no backing-store effect.
                }
            }
            _ => self.regs[(off >> 2) as usize & 0xFF] = value,
        }
    }

    /// Take the flash op an IP command triggered (for the bus to service).
    pub fn take_pending(&mut self) -> Option<FlashOp> {
        self.pending.take()
    }

    /// Stage bytes read from flash into the RX FIFO.
    pub fn stage_rx(&mut self, data: &[u8]) {
        self.rx.clear();
        self.rx.extend(data.iter().copied());
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

    /// Program a LUT sequence `seqid` and select it via IPCR0/IPCR1.
    fn arm_seq(f: &mut FlexSpi, seqid: u32, ops: &[u32], addr: u32, size: u32) {
        let base = 0x200 + seqid * 16; // 4 LUT words per sequence
        for (i, pair) in ops.chunks(2).enumerate() {
            let lo = pair[0] << 10;
            let hi = pair.get(1).map_or(0, |op| op << 10);
            f.write(base + i as u32 * 4, lo | hi << 16);
        }
        f.write(IPCR0, addr);
        f.write(IPCR1, size | (seqid << 16));
    }

    #[test]
    fn page_program_latches_addr_then_streams_data() {
        // The NXP HAL triggers IPCMD *before* pushing the payload; the write
        // must still land at IPCR0, not be misattributed to a later command.
        let mut f = FlexSpi::new(1);
        arm_seq(&mut f, 1, &[0x01, OP_RADDR, OP_WRITE], 0x0080_0000, 8);
        f.write(IPCMD, IPCMD_TRG);
        assert!(f.take_pending().is_none(), "no op until the data arrives");
        f.write(0x180, 0xddcc_bbaa); // IPTXFDR: first 4 bytes
        f.write(0x184, 0x4433_2211); // last 4 bytes -> finalize
        match f.take_pending() {
            Some(FlashOp::Program { addr, data }) => {
                assert_eq!(addr, 0x0080_0000);
                assert_eq!(data, [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44]);
            }
            other => panic!("expected a program at 0x800000, got {other:?}"),
        }
    }

    #[test]
    fn read_and_erase_classified_from_the_lut() {
        let mut f = FlexSpi::new(1);
        arm_seq(&mut f, 2, &[0x03, OP_RADDR, OP_READ], 0x0000_1234, 16);
        f.write(IPCMD, IPCMD_TRG);
        assert!(matches!(
            f.take_pending(),
            Some(FlashOp::Read {
                addr: 0x1234,
                size: 16
            })
        ));

        // A command carrying an address but no data movement is a sector erase.
        arm_seq(&mut f, 3, &[0x20, OP_RADDR], 0x0080_1000, 0);
        f.write(IPCMD, IPCMD_TRG);
        assert!(matches!(
            f.take_pending(),
            Some(FlashOp::Erase { addr: 0x0080_1000 })
        ));
    }

    #[test]
    fn config_write_without_address_touches_no_flash() {
        // WRSR-style: a WRITE sequence with no RADDR moves no backing store.
        let mut f = FlexSpi::new(1);
        arm_seq(&mut f, 1, &[0x01, OP_WRITE], 0, 1);
        f.write(IPCMD, IPCMD_TRG);
        f.write(0x180, 0x0000_0040); // status byte payload
        assert!(f.take_pending().is_none());
    }
}

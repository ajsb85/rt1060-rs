// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! USDHC — Ultra Secured Digital Host Controller (MIMXRT1062.h `USDHC_Type`;
//! RM §58). USDHC1/2 (IRQ 110/111). The SwiftIO Micro reads its user image
//! and assets from an SD card through this block.
//!
//! Models the command engine + PIO data path against an attachable
//! [`SdCard`]: a `CMD_XFR_TYP` write dispatches an SD command (index/argument),
//! the response is latched into `CMD_RSP0..3` and `INT_STATUS.CC` is set. A
//! read command fills a buffer the firmware drains through
//! `DATA_BUFF_ACC_PORT` (with `PRES_STATE.BREN` + `INT_STATUS.BRR`); a write
//! command consumes words written to the port into the card, block by block.
//! `PRES_STATE` reports a stable clock, inserted card, and no command/data
//! inhibit. ADMA/DMA (`DS_ADDR`/`ADMA_SYS_ADDR`), UHS tuning, and CMD12
//! auto-stop are ROADMAP items.
//!
//! Register map (offsets): BLK_ATT 0x04, CMD_ARG 0x08, CMD_XFR_TYP 0x0C,
//! CMD_RSP0..3 0x10..0x1C, DATA_BUFF_ACC_PORT 0x20, PRES_STATE 0x24,
//! SYS_CTRL 0x2C, INT_STATUS 0x30, MIX_CTRL 0x48.

use std::collections::VecDeque;

/// SD block size (fixed 512 for SDHC).
pub const BLOCK: usize = 512;

// PRES_STATE (0x24)
const PRES_CIHB: u32 = 1 << 0; // command inhibit (CMD)
const PRES_CDIHB: u32 = 1 << 1; // command inhibit (DATA)
const PRES_SDSTB: u32 = 1 << 3; // SD clock stable
const PRES_BWEN: u32 = 1 << 10; // buffer write enable
const PRES_BREN: u32 = 1 << 11; // buffer read enable
const PRES_CINST: u32 = 1 << 16; // card inserted

// INT_STATUS (0x30)
const INT_CC: u32 = 1 << 0; // command complete
const INT_TC: u32 = 1 << 1; // transfer complete
const INT_BWR: u32 = 1 << 4; // buffer write ready
const INT_BRR: u32 = 1 << 5; // buffer read ready
const INT_CTOE: u32 = 1 << 16; // command timeout error

/// An SD card backing a USDHC port. SDHC (block-addressed), 512-byte blocks.
pub struct SdCard {
    image: Vec<u8>,
    rca: u16,
}

impl SdCard {
    /// A card backed by `image` (padded up to a block boundary).
    pub fn new(mut image: Vec<u8>) -> Self {
        let rem = image.len() % BLOCK;
        if rem != 0 {
            image.resize(image.len() + (BLOCK - rem), 0);
        }
        Self { image, rca: 0 }
    }

    /// A blank card of `blocks` × 512 bytes.
    pub fn blank(blocks: usize) -> Self {
        Self {
            image: vec![0; blocks * BLOCK],
            rca: 0,
        }
    }

    pub fn block_count(&self) -> usize {
        self.image.len() / BLOCK
    }

    fn read_span(&self, block: u32, count: u32) -> Vec<u8> {
        let start = block as usize * BLOCK;
        let end = (start + count as usize * BLOCK).min(self.image.len());
        let mut v = self.image[start.min(self.image.len())..end].to_vec();
        v.resize(count as usize * BLOCK, 0); // zero-fill past the end
        v
    }

    fn write_block(&mut self, block: u32, data: &[u8]) {
        let start = block as usize * BLOCK;
        if start + BLOCK <= self.image.len() {
            self.image[start..start + BLOCK].copy_from_slice(&data[..BLOCK]);
        }
    }
}

/// The result of dispatching one SD command.
struct CmdResult {
    resp: [u32; 4],
    /// Bytes to stream back to the host (a read command).
    read: Vec<u8>,
    /// Number of blocks the host will write next (a write command).
    write_blocks: u32,
}

impl SdCard {
    /// Execute command `idx` with `arg`; `acmd` marks an application command
    /// (preceded by CMD55); `blkcnt` is the block count from `BLK_ATT`.
    fn command(&mut self, idx: u32, arg: u32, acmd: bool, blkcnt: u32) -> CmdResult {
        let mut r = CmdResult {
            resp: [0; 4],
            read: Vec::new(),
            write_blocks: 0,
        };
        let count = blkcnt.max(1);
        if acmd {
            match idx {
                41 => r.resp[0] = 0xC0FF_8000, // ACMD41: OCR, busy done + CCS (SDHC)
                6 => {}                        // ACMD6 SET_BUS_WIDTH
                51 => r.read = scr(),          // ACMD51 SEND_SCR (8 bytes)
                _ => {}
            }
            return r;
        }
        match idx {
            0 => {}                        // CMD0 GO_IDLE
            8 => r.resp[0] = arg & 0xFFF,  // CMD8 SEND_IF_COND: echo (0x1AA)
            55 => r.resp[0] = 0x0000_0120, // CMD55 APP_CMD: R1 (ready, APP_CMD)
            2 => r.resp = cid(),           // CMD2 ALL_SEND_CID (R2)
            3 => {
                self.rca = 0x0001;
                r.resp[0] = (u32::from(self.rca) << 16) | 0x0500; // R6
            }
            9 => r.resp = csd(self.block_count()), // CMD9 SEND_CSD (R2)
            7 | 16 | 12 | 13 | 6 => r.resp[0] = 0x0000_0900, // R1: tran state, ready
            17 | 18 => {
                let blocks = if idx == 17 { 1 } else { count };
                r.read = self.read_span(arg, blocks);
                r.resp[0] = 0x0000_0900;
            }
            24 | 25 => {
                r.write_blocks = if idx == 24 { 1 } else { count };
                r.resp[0] = 0x0000_0900;
            }
            _ => r.resp[0] = 0x0000_0900,
        }
        r
    }
}

/// SCR register (SD spec 3.0, 1-bit + 4-bit bus, physical spec 2.0).
fn scr() -> Vec<u8> {
    vec![0x02, 0x35, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00]
}

/// A plausible CID (R2), packed as the USDHC latches it into RSP3..RSP0.
fn cid() -> [u32; 4] {
    // Manufacturer 'RT', product "RT106", etc. — values are cosmetic; only
    // the block count in the CSD matters to the SDK's capacity math.
    [0x0000_0001, 0x3630_3154, 0x5254_2020, 0x0353_4400]
}

/// CSD version 2.0 (SDHC): C_SIZE derives capacity = (C_SIZE + 1) × 512 KB.
/// Packed as USDHC RSP3..RSP0 (the 120-bit CSD minus CRC, left-aligned).
fn csd(block_count: usize) -> [u32; 4] {
    // capacity_blocks = (C_SIZE + 1) * 1024  ⇒  C_SIZE = blocks/1024 - 1
    let c_size = (block_count / 1024).saturating_sub(1) as u32 & 0x003F_FFFF;
    // CSD_STRUCTURE=1 (v2) in the top byte; C_SIZE sits at bits [69:48] of the
    // 128-bit CSD. After the USDHC's 8-bit right-justify, C_SIZE lands in
    // RSP1[15:0]:RSP2[31:26] region — SDK reads it via SDMMC_ReadBits.
    let mut rsp = [0u32; 4];
    rsp[3] = 0x4000_0000; // CSD_STRUCTURE = 1 (SDHC/SDXC)
    rsp[1] = (c_size & 0xFFFF) << 16 | 0x5A80; // C_SIZE low + read/write params
    rsp[2] = 0x5B59_0000 | (c_size >> 16); // C_SIZE high bits
    rsp
}

pub struct Usdhc {
    pub index: u8,
    regs: [u32; 0x100], // 1 KiB register file (256 words)
    int_status: u32,
    /// Read data waiting for the host to drain via DATA_BUFF_ACC_PORT.
    rx: VecDeque<u32>,
    /// Write bytes accumulated from the host until a block is complete.
    wx: Vec<u8>,
    write_block: u32,
    write_blocks_left: u32,
    app_cmd: bool,
    card: Option<SdCard>,
}

impl Usdhc {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            regs: [0; 0x100],
            int_status: 0,
            rx: VecDeque::new(),
            wx: Vec::new(),
            write_block: 0,
            write_blocks_left: 0,
            app_cmd: false,
            card: None,
        }
    }

    /// Insert an SD card into this port.
    pub fn insert(&mut self, card: SdCard) {
        self.card = Some(card);
    }

    fn pres_state(&self) -> u32 {
        let mut s = PRES_SDSTB; // clock always stable
        if self.card.is_some() {
            s |= PRES_CINST; // card inserted
        }
        if !self.rx.is_empty() {
            s |= PRES_BREN;
        }
        if self.write_blocks_left > 0 {
            s |= PRES_BWEN;
        }
        // CIHB/CDIHB stay clear — commands complete instantly.
        let _ = (PRES_CIHB, PRES_CDIHB);
        s
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x10..=0x1C => self.regs[(offset >> 2) as usize & 0xFF], // CMD_RSP0..3
            0x20 => self.rx.pop_front().unwrap_or(0),                // DATA port
            0x24 => self.pres_state(),
            0x30 => self.int_status,
            _ => self.regs[(offset >> 2) as usize & 0xFF],
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x0C => {
                self.regs[3] = value; // stash CMD_XFR_TYP
                self.dispatch(value);
            }
            0x20 => self.push_write_word(value), // DATA port
            0x30 => self.int_status &= !value,   // INT_STATUS is W1C
            _ => self.regs[(offset >> 2) as usize & 0xFF] = value,
        }
    }

    fn dispatch(&mut self, xfr: u32) {
        let idx = (xfr >> 24) & 0x3F;
        let arg = self.regs[2]; // CMD_ARG at 0x08 → word index 2
        let blkcnt = (self.regs[1] >> 16) & 0xFFFF; // BLK_ATT.BLKCNT
        let Some(card) = self.card.as_mut() else {
            self.int_status |= INT_CTOE; // no card → timeout
            return;
        };
        let acmd = self.app_cmd && idx != 55;
        let res = card.command(idx, arg, acmd, blkcnt);
        self.app_cmd = idx == 55;

        self.regs[4] = res.resp[0]; // CMD_RSP0 @ 0x10
        self.regs[5] = res.resp[1];
        self.regs[6] = res.resp[2];
        self.regs[7] = res.resp[3];
        self.int_status |= INT_CC;

        if !res.read.is_empty() {
            self.rx.clear();
            for chunk in res.read.chunks(4) {
                let mut w = [0u8; 4];
                w[..chunk.len()].copy_from_slice(chunk);
                self.rx.push_back(u32::from_le_bytes(w));
            }
            self.int_status |= INT_BRR;
        }
        if res.write_blocks > 0 {
            self.write_block = arg;
            self.write_blocks_left = res.write_blocks;
            self.wx.clear();
            self.int_status |= INT_BWR;
        }
        if res.read.is_empty() && res.write_blocks == 0 {
            self.int_status |= INT_TC; // no data phase → transfer complete now
        } else if !res.read.is_empty() {
            self.int_status |= INT_TC; // read buffer fully staged
        }
    }

    fn push_write_word(&mut self, word: u32) {
        if self.write_blocks_left == 0 {
            return;
        }
        self.wx.extend_from_slice(&word.to_le_bytes());
        while self.wx.len() >= BLOCK {
            let block: Vec<u8> = self.wx.drain(..BLOCK).collect();
            if let Some(card) = self.card.as_mut() {
                card.write_block(self.write_block, &block);
            }
            self.write_block += 1;
            self.write_blocks_left -= 1;
            if self.write_blocks_left == 0 {
                self.int_status |= INT_TC;
                break;
            }
        }
    }

    pub fn irq_pending(&self) -> bool {
        // INT_SIGNAL_EN at 0x38 → word index 14.
        self.int_status & self.regs[14] != 0
    }
}

impl Default for Usdhc {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_with_pattern() -> SdCard {
        // Two blocks: block 0 = 0x00,0x01,...; block 1 = 0xFF,0xFE,...
        let mut img = vec![0u8; 2 * BLOCK];
        for (i, b) in img.iter_mut().enumerate() {
            *b = if i < BLOCK { i as u8 } else { !(i as u8) };
        }
        SdCard::new(img)
    }

    fn issue(u: &mut Usdhc, idx: u32, arg: u32) {
        u.write(0x08, arg); // CMD_ARG
        u.write(0x0C, idx << 24); // CMD_XFR_TYP
    }

    #[test]
    fn present_state_reports_inserted_card() {
        let mut u = Usdhc::new(1);
        assert_eq!(u.read(0x24) & PRES_CINST, 0, "no card yet");
        u.insert(card_with_pattern());
        assert_ne!(u.read(0x24) & PRES_CINST, 0, "card inserted");
        assert_ne!(u.read(0x24) & PRES_SDSTB, 0, "clock stable");
    }

    #[test]
    fn init_sequence_completes() {
        let mut u = Usdhc::new(1);
        u.insert(card_with_pattern());
        issue(&mut u, 0, 0); // CMD0
        issue(&mut u, 8, 0x1AA); // CMD8
        assert_eq!(u.read(0x10) & 0xFFF, 0x1AA, "CMD8 echoes the check pattern");
        issue(&mut u, 55, 0); // CMD55 (APP_CMD)
        issue(&mut u, 41, 0x40FF_8000); // ACMD41
        assert_ne!(u.read(0x10) & (1 << 31), 0, "OCR busy-done");
        assert_ne!(u.read(0x10) & (1 << 30), 0, "CCS = SDHC");
        assert_ne!(u.read(0x30) & INT_CC, 0, "command complete");
    }

    #[test]
    fn read_single_block_through_data_port() {
        let mut u = Usdhc::new(1);
        u.insert(card_with_pattern());
        u.write(0x04, 1 << 16 | BLOCK as u32); // BLK_ATT: 1 block × 512
        issue(&mut u, 17, 1); // CMD17 READ_SINGLE_BLOCK, block 1
        assert_ne!(u.read(0x24) & PRES_BREN, 0, "buffer read ready");
        assert_ne!(u.read(0x30) & INT_BRR, 0);
        // Block 1 was 0xFF,0xFE,... → first word LE = 0xFCFDFEFF.
        assert_eq!(u.read(0x20), 0xFCFD_FEFF);
    }

    #[test]
    fn write_block_reaches_the_card() {
        let mut u = Usdhc::new(1);
        u.insert(SdCard::blank(4));
        u.write(0x04, 1 << 16 | BLOCK as u32);
        issue(&mut u, 24, 2); // CMD24 WRITE_BLOCK, block 2
        assert_ne!(u.read(0x24) & PRES_BWEN, 0, "buffer write enabled");
        for i in 0..(BLOCK / 4) {
            u.write(0x20, 0xA5A5_0000 | i as u32);
        }
        assert_ne!(u.read(0x30) & INT_TC, 0, "transfer complete");
        // Read it back.
        u.write(0x04, 1 << 16 | BLOCK as u32);
        issue(&mut u, 17, 2);
        assert_eq!(u.read(0x20), 0xA5A5_0000);
    }

    #[test]
    fn no_card_times_out() {
        let mut u = Usdhc::new(1);
        issue(&mut u, 0, 0);
        assert_ne!(u.read(0x30) & INT_CTOE, 0, "command timeout without a card");
    }
}

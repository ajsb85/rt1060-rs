// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! LPI2C — Low-Power I²C master (MIMXRT1062.h `LPI2C_Type`; RM §47).
//!
//! Models the master command/data path the SwiftIO HAL and MadDrivers use:
//! firmware pushes command+data words into `MTDR` (START/addr, TX, RX, STOP)
//! and pops received bytes from `MRDR`. A transaction is dispatched to an
//! attached [`I2cDevice`] by 7-bit address; an unaddressed transfer raises
//! `MSR.NDF` (NACK). Slave mode, DMA, and bus-timing are ROADMAP items.
//!
//! Register map (offsets): MCR 0x10, MSR 0x14, MIER 0x18, MCFGR0 0x20,
//! MFSR 0x5C, MTDR 0x60, MRDR 0x70. `MTDR.CMD` = bits [10:8], data = [7:0].
//! Device hooks are off the CPU execution path, so a boxed trait object is
//! acceptable here (it is only touched during an I²C transaction).

use std::collections::VecDeque;

// MCR (0x10)
const MCR_MEN: u32 = 1 << 0; // master enable
const MCR_RST: u32 = 1 << 1; // software reset
const MCR_RTF: u32 = 1 << 8; // reset TX FIFO
const MCR_RRF: u32 = 1 << 9; // reset RX FIFO
// MSR (0x14)
const MSR_TDF: u32 = 1 << 0; // transmit data flag (FIFO below watermark)
const MSR_RDF: u32 = 1 << 1; // receive data flag (FIFO above watermark)
const MSR_EPF: u32 = 1 << 8; // end packet flag
const MSR_SDF: u32 = 1 << 9; // STOP detect flag
const MSR_NDF: u32 = 1 << 10; // NACK detect flag
const MSR_MBF: u32 = 1 << 24; // master busy flag
// MRDR (0x70)
const MRDR_RXEMPTY: u32 = 1 << 14;

// MTDR command field (bits [10:8]).
const CMD_TX: u32 = 0b000; // transmit DATA
const CMD_RX: u32 = 0b001; // receive DATA+1 bytes
const CMD_STOP: u32 = 0b010; // generate STOP
const CMD_RX_DISCARD: u32 = 0b011; // receive and discard DATA+1 bytes
const CMD_START: u32 = 0b100; // (repeated) START + transmit address
const CMD_START_NACK: u32 = 0b101; // START + address, expect NACK

/// An I²C peripheral attached to a bus. Off the CPU hot path — only touched
/// during a transaction.
pub trait I2cDevice {
    /// 7-bit address this device answers to.
    fn address(&self) -> u8;
    /// START addressed to this device; `read` is the R/W bit. Return the ACK.
    fn start(&mut self, read: bool) -> bool {
        let _ = read;
        true
    }
    /// Master writes a byte; return the ACK.
    fn write(&mut self, byte: u8) -> bool;
    /// Master reads a byte.
    fn read(&mut self) -> u8;
    /// STOP condition.
    fn stop(&mut self) {}
}

pub struct LpI2c {
    pub index: u8,
    /// Host-bridged mode for an embedding wiring engine: `MTDR` commands are
    /// recorded in the bus-event trace instead of dispatching to the locally
    /// attached `devices`, and the host answers via [`LpI2c::rx_push`] /
    /// [`LpI2c::set_nack`]. STOP status (SDF/EPF) stays local — firmware
    /// polls it.
    pub external: bool,
    mcr: u32,
    mier: u32,
    mder: u32,
    /// Sticky status bits the driver clears via W1C (SDF/NDF/EPF/…).
    msr_sticky: u32,
    rx: VecDeque<u8>,
    devices: Vec<Box<dyn I2cDevice + Send>>,
    /// Index into `devices` of the currently addressed target.
    active: Option<usize>,
    /// External-mode master-busy latch (START..STOP).
    ext_busy: bool,
}

impl LpI2c {
    pub fn new(index: u8) -> Self {
        Self {
            index,
            external: false,
            mcr: 0,
            mier: 0,
            mder: 0,
            msr_sticky: 0,
            rx: VecDeque::new(),
            devices: Vec::new(),
            active: None,
            ext_busy: false,
        }
    }

    /// Attach a device to this bus (builder-style for tests / board setup).
    pub fn attach(&mut self, dev: Box<dyn I2cDevice + Send>) {
        self.devices.push(dev);
    }

    /// External-mode host delivers a byte the kernel's device read returned.
    pub fn rx_push(&mut self, byte: u8) {
        self.rx.push_back(byte);
    }

    /// External-mode host reflects an address/data NACK (`MSR.NDF`).
    pub fn set_nack(&mut self) {
        self.msr_sticky |= MSR_NDF;
        self.ext_busy = false;
    }

    fn find(&self, addr7: u8) -> Option<usize> {
        self.devices.iter().position(|d| d.address() == addr7)
    }

    fn msr(&self) -> u32 {
        let mut s = self.msr_sticky;
        if self.mcr & MCR_MEN != 0 {
            s |= MSR_TDF; // TX FIFO is always drained (instant transfer)
        }
        if !self.rx.is_empty() {
            s |= MSR_RDF;
        }
        if self.active.is_some() || self.ext_busy {
            s |= MSR_MBF;
        }
        s
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => 0x0101_0003, // VERID
            0x04 => 0x0000_0202, // PARAM: 4-entry TX/RX FIFOs
            0x10 => self.mcr,
            0x14 => self.msr(),
            0x18 => self.mier,
            0x5C => (self.rx.len() as u32) << 16, // MFSR: RXCOUNT [18:16]
            0x70 => match self.rx.pop_front() {
                Some(b) => u32::from(b),
                None => MRDR_RXEMPTY,
            },
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x10 => {
                if value & MCR_RST != 0 {
                    self.reset();
                }
                if value & MCR_RRF != 0 {
                    self.rx.clear();
                }
                let _ = MCR_RTF; // TX FIFO is never backed up (instant TX)
                self.mcr = value & !(MCR_RST | MCR_RTF | MCR_RRF);
            }
            0x14 => self.msr_sticky &= !(value & (MSR_EPF | MSR_SDF | MSR_NDF)),
            0x18 => self.mier = value,
            0x1C => self.mder = value, // MDER: DMA enable
            0x60 => self.command((value >> 8) & 0x7, value as u8),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.mcr = 0;
        self.mier = 0;
        self.msr_sticky = 0;
        self.rx.clear();
        self.active = None;
        self.ext_busy = false;
    }

    /// Execute one `MTDR` command word.
    fn command(&mut self, cmd: u32, data: u8) {
        if self.external {
            // The host observes the command through the bus-event trace and
            // answers with `rx_push`/`set_nack`; only the locally polled
            // START/STOP status is kept here.
            match cmd {
                CMD_START | CMD_START_NACK => self.ext_busy = true,
                CMD_STOP => {
                    self.ext_busy = false;
                    self.msr_sticky |= MSR_SDF | MSR_EPF;
                }
                _ => {}
            }
            return;
        }
        match cmd {
            CMD_START | CMD_START_NACK => {
                let addr7 = data >> 1;
                let read = data & 1 != 0;
                match self.find(addr7) {
                    Some(i) if self.devices[i].start(read) => self.active = Some(i),
                    _ => {
                        // Unaddressed / NACKed target.
                        self.active = None;
                        self.msr_sticky |= MSR_NDF;
                    }
                }
            }
            CMD_TX => {
                if let Some(i) = self.active {
                    if !self.devices[i].write(data) {
                        self.msr_sticky |= MSR_NDF;
                    }
                } else {
                    self.msr_sticky |= MSR_NDF;
                }
            }
            CMD_RX | CMD_RX_DISCARD => {
                let n = u32::from(data) + 1;
                for _ in 0..n {
                    if let Some(i) = self.active {
                        let b = self.devices[i].read();
                        if cmd == CMD_RX {
                            self.rx.push_back(b);
                        }
                    }
                }
            }
            CMD_STOP => {
                if let Some(i) = self.active {
                    self.devices[i].stop();
                }
                self.active = None;
                self.msr_sticky |= MSR_SDF | MSR_EPF;
            }
            _ => {}
        }
    }

    pub fn irq_pending(&self) -> bool {
        self.msr() & self.mier != 0
    }

    /// DMA request (rx-driven): `MDER.RDDE` set and a received byte waiting.
    pub fn dma_request(&self) -> bool {
        self.mder & 0x2 != 0 && !self.rx.is_empty()
    }
}

/// A simple register-file I²C device (EEPROM/sensor shape): the first write
/// after START sets the register pointer; further writes store, reads return
/// `regs[ptr]` with auto-increment. Handy as a test/demo target for
/// MadDrivers.
pub struct MemI2cDevice {
    addr: u8,
    regs: [u8; 256],
    ptr: u8,
    expect_ptr: bool,
}

impl MemI2cDevice {
    pub fn new(addr: u8) -> Self {
        Self {
            addr,
            regs: [0; 256],
            ptr: 0,
            expect_ptr: true,
        }
    }

    /// Pre-seed a register (e.g. a sensor WHO_AM_I value).
    pub fn with(mut self, reg: u8, value: u8) -> Self {
        self.regs[reg as usize] = value;
        self
    }
}

impl I2cDevice for MemI2cDevice {
    fn address(&self) -> u8 {
        self.addr
    }
    fn start(&mut self, read: bool) -> bool {
        // A write transaction begins with a register-pointer byte; a read
        // continues from the current pointer.
        self.expect_ptr = !read;
        true
    }
    fn write(&mut self, byte: u8) -> bool {
        if self.expect_ptr {
            self.ptr = byte;
            self.expect_ptr = false;
        } else {
            self.regs[self.ptr as usize] = byte;
            self.ptr = self.ptr.wrapping_add(1);
        }
        true
    }
    fn read(&mut self) -> u8 {
        let b = self.regs[self.ptr as usize];
        self.ptr = self.ptr.wrapping_add(1);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus_with_sensor() -> LpI2c {
        let mut i2c = LpI2c::new(1);
        i2c.write(0x10, MCR_MEN); // MCR.MEN
        // Sensor at 0x1D with WHO_AM_I = 0x49 at register 0x0F.
        i2c.attach(Box::new(MemI2cDevice::new(0x1D).with(0x0F, 0x49)));
        i2c
    }

    #[test]
    fn write_then_read_register() {
        let mut i2c = bus_with_sensor();
        // START+addr (write): 0x1D << 1 | 0.
        i2c.write(0x60, (CMD_START << 8) | (0x1Du32 << 1));
        assert!(i2c.msr() & MSR_NDF == 0, "device ACKed");
        // Set register pointer to 0x0F.
        i2c.write(0x60, (CMD_TX << 8) | 0x0F);
        // Repeated START+addr (read): 0x1D << 1 | 1.
        i2c.write(0x60, (CMD_START << 8) | ((0x1Du32 << 1) | 1));
        // Receive 1 byte (DATA+1 = 1).
        i2c.write(0x60, CMD_RX << 8);
        assert_ne!(i2c.msr() & MSR_RDF, 0, "RX data available");
        assert_eq!(i2c.read(0x70) & 0xFF, 0x49, "WHO_AM_I");
        i2c.write(0x60, CMD_STOP << 8);
        assert_ne!(i2c.msr() & MSR_SDF, 0, "STOP detected");
    }

    #[test]
    fn unaddressed_target_nacks() {
        let mut i2c = bus_with_sensor();
        i2c.write(0x60, (CMD_START << 8) | (0x42u32 << 1)); // no device
        assert_ne!(i2c.msr() & MSR_NDF, 0, "NACK on missing device");
    }

    #[test]
    fn rx_empty_reads_flag() {
        let mut i2c = bus_with_sensor();
        assert_ne!(i2c.read(0x70) & MRDR_RXEMPTY, 0);
    }

    #[test]
    fn irq_on_nack_when_enabled() {
        let mut i2c = bus_with_sensor();
        i2c.write(0x18, MSR_NDF); // MIER: NACK interrupt
        i2c.write(0x60, (CMD_START << 8) | (0x42u32 << 1));
        assert!(i2c.irq_pending());
    }

    #[test]
    fn external_mode_defers_the_transaction_to_the_host() {
        let mut i2c = bus_with_sensor();
        i2c.external = true;
        // START to the locally attached sensor: not dispatched, not NACKed.
        i2c.write(0x60, (CMD_START << 8) | (0x1Du32 << 1));
        assert_eq!(i2c.msr() & MSR_NDF, 0, "host decides the ACK");
        assert_ne!(i2c.msr() & MSR_MBF, 0, "master busy after START");
        // RX command: nothing self-satisfies; the host pushes the byte.
        i2c.write(0x60, CMD_RX << 8);
        assert_eq!(i2c.msr() & MSR_RDF, 0, "no self-satisfied RX");
        i2c.rx_push(0x49);
        assert_ne!(i2c.msr() & MSR_RDF, 0);
        assert_eq!(i2c.read(0x70) & 0xFF, 0x49);
        // STOP status stays local — firmware polls SDF/EPF.
        i2c.write(0x60, CMD_STOP << 8);
        assert_ne!(i2c.msr() & MSR_SDF, 0);
        assert_eq!(i2c.msr() & MSR_MBF, 0, "bus idle after STOP");
    }

    #[test]
    fn external_mode_host_nack_raises_ndf() {
        let mut i2c = bus_with_sensor();
        i2c.external = true;
        i2c.write(0x18, MSR_NDF); // MIER: NACK interrupt
        i2c.write(0x60, (CMD_START << 8) | (0x42u32 << 1));
        assert!(!i2c.irq_pending(), "no local NACK in external mode");
        i2c.set_nack();
        assert_ne!(i2c.msr() & MSR_NDF, 0);
        assert!(i2c.irq_pending());
        assert_eq!(i2c.msr() & MSR_MBF, 0, "NACK aborts the transaction");
    }
}

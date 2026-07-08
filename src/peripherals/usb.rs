// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! USB OTG — ChipIdea/EHCI-style dual-role controller (MIMXRT1062.h
//! `USB_Type`; RM §42). Two controllers (USB1 / USB2) share one 16 KiB
//! routing window: USB1 at window offset `0x000`, USB2 at `0x200`. OTG1 =
//! IRQ 113, OTG2 = IRQ 112. On the SwiftIO the USB-C port (OTG1) is the
//! `mm download` / CDC-ACM console bridge.
//!
//! This models the **controller register block** — enough that the device
//! stack's init runs without hanging on unmodeled registers:
//!
//! - `USBCMD.RST` (controller reset) self-clears, so the reset spin-loop
//!   `while (USBCMD & RST)` terminates;
//! - `USBSTS` is write-1-clear;
//! - `PORTSC1` reports a connected high-speed port so the device sees a live
//!   bus;
//! - `USBMODE`, `DEVICEADDR`, `ENDPTLISTADDR`, and the endpoint control
//!   registers read back what firmware wrote.
//!
//! The transfer engine — queue heads / dTDs in memory, SETUP handling, full
//! CDC-ACM enumeration — is a ROADMAP item; until then enumeration idles
//! (waiting for bus events this model does not yet generate) rather than
//! hanging.
//!
//! Register offsets (per controller): USBCMD 0x140, USBSTS 0x144,
//! USBINTR 0x148, DEVICEADDR 0x154, PORTSC1 0x184, USBMODE 0x1A8.

const USBCMD: u32 = 0x140;
const USBSTS: u32 = 0x144;
const USBINTR: u32 = 0x148;
const PORTSC1: u32 = 0x184;

const CMD_RST: u32 = 1 << 1; // controller reset (self-clearing)
const PORTSC_CCS: u32 = 1 << 0; // current connect status
const PORTSC_HS: u32 = 2 << 26; // PSPD = high speed

/// One ChipIdea controller's register file (0x200 bytes = 128 words).
struct UsbCtrl {
    regs: [u32; 128],
}

impl UsbCtrl {
    fn new() -> Self {
        Self { regs: [0; 128] }
    }

    #[inline]
    fn idx(off: u32) -> usize {
        ((off & 0x1FF) >> 2) as usize
    }

    fn read(&self, off: u32) -> u32 {
        match off & 0x1FF {
            USBCMD => self.regs[Self::idx(USBCMD)] & !CMD_RST, // RST self-cleared
            PORTSC1 => self.regs[Self::idx(PORTSC1)] | PORTSC_CCS | PORTSC_HS,
            _ => self.regs[Self::idx(off)],
        }
    }

    fn write(&mut self, off: u32, value: u32) {
        match off & 0x1FF {
            USBCMD => self.regs[Self::idx(USBCMD)] = value & !CMD_RST,
            USBSTS => self.regs[Self::idx(USBSTS)] &= !value, // W1C
            _ => self.regs[Self::idx(off)] = value,
        }
    }

    fn irq_pending(&self) -> bool {
        self.regs[Self::idx(USBSTS)] & self.regs[Self::idx(USBINTR)] != 0
    }
}

/// Both USB controllers sharing the `0x402E_0000` window.
pub struct Usb {
    ctrl: [UsbCtrl; 2],
}

impl Usb {
    pub fn new() -> Self {
        Self {
            ctrl: [UsbCtrl::new(), UsbCtrl::new()],
        }
    }

    /// USB1 for window offsets `< 0x200`, USB2 for `>= 0x200`.
    #[inline]
    fn split(off: u32) -> usize {
        (off >= 0x200) as usize
    }

    pub fn read(&mut self, off: u32) -> u32 {
        self.ctrl[Self::split(off)].read(off)
    }

    pub fn write(&mut self, off: u32, value: u32) {
        self.ctrl[Self::split(off)].write(off, value);
    }

    /// IRQ pending for controller `n` (0 = USB1/OTG1, 1 = USB2/OTG2).
    pub fn irq_pending(&self, n: usize) -> bool {
        self.ctrl[n].irq_pending()
    }
}

impl Default for Usb {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_reset_self_clears() {
        let mut usb = Usb::new();
        usb.write(USBCMD, CMD_RST | 0x1); // USB1: RST + RS
        assert_eq!(usb.read(USBCMD) & CMD_RST, 0, "RST reads back clear");
        assert_ne!(usb.read(USBCMD) & 0x1, 0, "RS (run) sticks");
    }

    #[test]
    fn port_reports_connected_high_speed() {
        let mut usb = Usb::new();
        let p = usb.read(PORTSC1);
        assert_ne!(p & PORTSC_CCS, 0, "connect status set");
        assert_eq!((p >> 26) & 0x3, 2, "high-speed port");
    }

    #[test]
    fn usbsts_is_write_one_clear_and_gates_irq() {
        let mut usb = Usb::new();
        // Fabricate a pending status + enable it.
        usb.ctrl[0].regs[UsbCtrl::idx(USBSTS)] = 0x41; // UI + URI
        usb.write(USBINTR, 0x41);
        assert!(usb.irq_pending(0));
        usb.write(USBSTS, 0x41); // W1C
        assert!(!usb.irq_pending(0));
    }

    #[test]
    fn usb1_and_usb2_are_independent() {
        let mut usb = Usb::new();
        usb.write(0x200 + 0x154, 0x1234_5678); // USB2 DEVICEADDR
        assert_eq!(usb.read(0x200 + 0x154), 0x1234_5678);
        assert_eq!(usb.read(0x154), 0, "USB1 unaffected");
    }
}

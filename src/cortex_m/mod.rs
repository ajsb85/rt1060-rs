//! Cortex-M7 (Armv7E-M) core — the i.MX RT1060's CPU (MIMXRT1062, M7 r1p2).
//!
//! Ported from the mg24-rs `cortex_m` core (a Cortex-M33 / Armv8-M Mainline
//! port of rp2350-rs, same author, MIT). The M7 runs the same Thumb-2
//! mainline + DSP instruction set, so the decode/execute path is identical;
//! the M7 is Armv7E-M, so it has no TrustZone/SAU (the SAU registers below
//! are harmless stored-readback holdovers and never gate access) but keeps
//! the MPU (PMSAv7, 16 regions) and an optional FPv5-D16 FPU. i.MX RT1060
//! adaptations: 158 external IRQs ([`IrqMask`], 5 words), M7 CPUID, and a
//! reset VTOR that the SoC points at the loaded image's vector table.
//!
//! The core is register state only; every memory access goes through the
//! [`Bus`] borrowed for the duration of one [`CortexM7::step`]. The PPB
//! region (`0xE000_0000+`) is core-local and intercepted here — it never
//! reaches the bus.

#[cfg(test)]
pub mod asm;
pub mod decoded;
mod fpu;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_exc;
#[cfg(test)]
mod tests_wide;
mod wide;

/// Memory seen by the core. `SystemBus` implements this for the real chip;
/// tests use a flat RAM implementation.
pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, value: u8);
    fn write16(&mut self, addr: u32, value: u16);
    fn write32(&mut self, addr: u32, value: u32);

    /// Fetch the (possibly cached) decoded instruction at `addr`. Buses
    /// with a decode cache override this; the default decodes on the fly.
    fn fetch_op(&mut self, addr: u32) -> crate::cortex_m::decoded::DecodedOp {
        let hw1 = self.read16(addr);
        let hw2 = if hw1 >= 0xe800 {
            self.read16(addr.wrapping_add(2))
        } else {
            0
        };
        crate::cortex_m::decoded::DecodedOp::decode(hw1, hw2, addr)
    }

    /// Coprocessor writes (MCR/MCRR). The MG24 has no bus-visible
    /// coprocessors (cp10/11, the FPU, is handled in-core); buses that
    /// don't model coprocessors ignore these.
    fn mcr(&mut self, _cp: u32, _opc1: u32, _crn: u32, _crm: u32, _opc2: u32, _value: u32) {}
    fn mcrr(&mut self, _cp: u32, _opc1: u32, _crm: u32, _lo: u32, _hi: u32) {}
    /// Coprocessor reads (MRC/MRRC); (0, 0) when unimplemented.
    fn mrc(&mut self, _cp: u32, _opc1: u32, _crn: u32, _crm: u32, _opc2: u32) -> u32 {
        0
    }
    fn mrrc(&mut self, _cp: u32, _opc1: u32, _crm: u32) -> (u32, u32) {
        (0, 0)
    }

    /// Level-sensitive IRQ lines currently asserted (bit n = IRQ n per
    /// efr32mg24b220f1536im48.h `IRQn_Type`). Sampled every step into the
    /// NVIC pending register. 158 lines (0..=157), hence [`IrqMask`].
    fn irq_lines(&mut self) -> IrqMask {
        IrqMask::ZERO
    }
}

pub const SP: usize = 13;
pub const LR: usize = 14;
pub const PC: usize = 15;

/// Why the core stopped executing on its own accord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakCause {
    /// BKPT instruction; payload is imm8.
    Bkpt(u8),
    /// Permanently undefined (UDF); payload is imm8.
    Udf(u8),
    /// An encoding this emulator does not implement yet (payload: the
    /// halfword(s), first in the low 16 bits).
    Unimplemented(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpSel {
    Main,
    Process,
}

pub struct CortexM7 {
    /// R0-R12, SP (banked value currently active), LR, PC.
    pub regs: [u32; 16],
    /// The inactive stack pointer (MSP when CONTROL.SPSEL=1, else PSP).
    pub sp_bank: u32,
    pub sp_sel: SpSel,
    // APSR flags, kept unpacked for cheap updates.
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    /// APSR.Q sticky saturation flag (bit 27; set by SSAT/USAT).
    pub q: bool,
    /// APSR.GE[3:0] greater-than-or-equal flags (bits 19:16; set by the DSP
    /// parallel add/sub instructions, consumed by SEL).
    pub ge: u8,
    /// Exception number currently active (IPSR); 0 in thread mode.
    pub ipsr: u16,
    /// EPSR.IT/ICI state, ITSTATE encoding (0 = not in an IT block).
    pub it_state: u8,
    pub primask: bool,
    pub basepri: u8,
    pub faultmask: bool,
    /// CONTROL.nPRIV (bit 0); SPSEL is tracked via `sp_sel`.
    pub npriv: bool,
    /// SCB VTOR (PPB 0xE000ED08).
    pub vtor: u32,
    // NVIC state for the 158 i.MX RT1060 external interrupts
    // (MIMXRT1062.h IRQn_Type, last = GPIO6_7_8_9_IRQn = 157): 158 > 128,
    // so a plain u128 will not hold every line — see [`IrqMask`].
    pub nvic_enable: IrqMask,
    /// Latched pends: ISPR writes and (for edges) past line activity.
    pub nvic_pending: IrqMask,
    /// Live level-sensitive input lines, sampled each step. Per Armv7E-M,
    /// a level interrupt that deasserts before being taken is no longer
    /// pending — so lines are OR'd at use, never latched into
    /// `nvic_pending`.
    pub irq_level: IrqMask,
    pub nvic_prio: [u8; NUM_IRQS],
    /// SHPR1-3: priorities for system handlers 4-15.
    pub shpr: [u32; 3],
    pub icsr_pendsv: bool,
    pub icsr_pendst: bool,
    /// Set when a WFI/WFE hint executes; the run loop consumes it to
    /// fast-forward idle time (EM1). Purely advisory.
    pub wfi_hint: bool,
    /// Set by an AIRCR.SYSRESETREQ write (NVIC_SystemReset); the chip
    /// runner consumes it and performs the reset (EMU cause SYSREQ).
    pub sysreset_request: bool,
    // SysTick.
    pub syst_csr: u32,
    pub syst_rvr: u32,
    pub syst_cvr: u32,
    pub scr: u32,
    pub shcsr: u32,
    pub cpacr: u32,
    // MPU (PMSAv8, 8 regions): stored-readback only, no enforcement.
    // EFR32MG24: sl_mpu_disable_execute_from_ram runs at boot (every
    // Arduino/SDK sl_system_init) and must see its writes stick.
    pub mpu_ctrl: u32,
    pub mpu_rnr: u32,
    pub mpu_rbar: u32,
    pub mpu_rlar: u32,
    /// MPU_MAIR0/1.
    pub mpu_mair: [u32; 2],
    // SAU (8 regions): TrustZone-lite per docs/DESIGN.md §2 — registers
    // accept and read back (boot code sets SAU_CTRL.ALLNS), but security
    // state never gates memory access.
    pub sau_ctrl: u32,
    pub sau_rnr: u32,
    pub sau_rbar: u32,
    pub sau_rlar: u32,
    /// FPv5-SP-D16 extension registers S0-S31 and FPSCR.
    pub fpregs: [u32; 32],
    pub fpscr: u32,
    /// EXC_RETURN value captured by a branch in handler mode; the step
    /// loop performs the actual return (needs bus access).
    exc_return_pending: Option<u32>,
    /// Monotonic instruction/cycle counter (1 per instruction for now).
    pub cycles: u64,
    /// Set when the core hits BKPT/UDF/unimplemented; the runner decides
    /// what to do (GDB stop, test failure, ...). Cleared by the runner.
    pub break_cause: Option<BreakCause>,
}

/// External interrupt count on the i.MX RT1060 (MIMXRT1062.h `IRQn_Type`,
/// last external line = `GPIO6_7_8_9_IRQn` = 157, so 158 lines). The NVIC
/// register banks are sized to 160 (5 × 32-bit words).
pub const NUM_IRQS: usize = 158;

/// Number of 32-bit words spanning the NVIC banks: ceil(158 / 32) = 5.
pub const IRQ_WORDS: usize = 5;

/// A bitset of the 158 external interrupt lines. Replaces the single-word
/// `u128` masks used by the M0+/M33 ports (76 IRQs fit a u128; 158 do not).
/// Word `w` bit `b` corresponds to IRQ `32*w + b`, matching the ISER/ICER/
/// ISPR/ICPR/IABR register banks 1:1 (bank word index = addr[4:2]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IrqMask(pub [u32; IRQ_WORDS]);

impl Default for IrqMask {
    fn default() -> Self {
        Self::ZERO
    }
}

impl IrqMask {
    pub const ZERO: Self = IrqMask([0; IRQ_WORDS]);

    /// Valid-bit mask for word `w`: the top word only carries IRQs 128..157
    /// (30 bits); bits for nonexistent IRQs are RAZ/WI.
    #[inline(always)]
    const fn word_mask(w: usize) -> u32 {
        let base = (w * 32) as u32;
        if base >= NUM_IRQS as u32 {
            0
        } else if base + 32 <= NUM_IRQS as u32 {
            0xFFFF_FFFF
        } else {
            (1u32 << (NUM_IRQS as u32 - base)) - 1
        }
    }

    #[inline(always)]
    pub fn word(&self, w: usize) -> u32 {
        self.0[w]
    }

    #[inline(always)]
    pub fn test(&self, irq: u32) -> bool {
        (self.0[(irq >> 5) as usize] >> (irq & 31)) & 1 != 0
    }

    #[inline(always)]
    pub fn set(&mut self, irq: u32) {
        if (irq as usize) < NUM_IRQS {
            self.0[(irq >> 5) as usize] |= 1 << (irq & 31);
        }
    }

    #[inline(always)]
    pub fn clear(&mut self, irq: u32) {
        if (irq as usize) < NUM_IRQS {
            self.0[(irq >> 5) as usize] &= !(1 << (irq & 31));
        }
    }

    /// OR `value` into word `w`, masking off RAZ/WI bits (ISER/ISPR set).
    #[inline(always)]
    pub fn set_word(&mut self, w: usize, value: u32) {
        self.0[w] |= value & Self::word_mask(w);
    }

    /// AND-NOT `value` out of word `w` (ICER/ICPR clear).
    #[inline(always)]
    pub fn clear_word(&mut self, w: usize, value: u32) {
        self.0[w] &= !value;
    }

    #[inline(always)]
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    #[inline(always)]
    pub fn and(&self, other: &Self) -> Self {
        IrqMask(std::array::from_fn(|i| self.0[i] & other.0[i]))
    }

    #[inline(always)]
    pub fn or(&self, other: &Self) -> Self {
        IrqMask(std::array::from_fn(|i| self.0[i] | other.0[i]))
    }

    /// Lowest set IRQ number, or `None` if empty. Ties in the exception
    /// selector go to the lowest number, so scan words low-to-high.
    #[inline(always)]
    pub fn lowest_set(&self) -> Option<u32> {
        for (i, &word) in self.0.iter().enumerate() {
            if word != 0 {
                return Some(i as u32 * 32 + word.trailing_zeros());
            }
        }
        None
    }
}

/// Word index of the NVIC bank an ISER/ICER/ISPR/ICPR access selects. The
/// banks are 32-byte-strided; the word index is addr[4:2] (0..=4).
#[inline(always)]
fn nvic_word_index(addr: u32) -> usize {
    ((addr >> 2) & 0x7) as usize
}

impl Default for CortexM7 {
    fn default() -> Self {
        Self::new()
    }
}

/// `AddWithCarry()` from the Arm ARM pseudocode: returns (result, carry, overflow).
#[inline(always)]
pub fn add_with_carry(x: u32, y: u32, carry_in: bool) -> (u32, bool, bool) {
    let (r1, c1) = x.overflowing_add(y);
    let (result, c2) = r1.overflowing_add(carry_in as u32);
    let carry = c1 | c2;
    let overflow = ((x ^ result) & (y ^ result)) >> 31 != 0;
    (result, carry, overflow)
}

impl CortexM7 {
    pub fn new() -> Self {
        CortexM7 {
            regs: [0; 16],
            sp_bank: 0,
            sp_sel: SpSel::Main,
            n: false,
            z: false,
            c: false,
            v: false,
            q: false,
            ge: 0,
            ipsr: 0,
            it_state: 0,
            primask: false,
            basepri: 0,
            faultmask: false,
            npriv: false,
            // i.MX RT1060 boots from the mask ROM (0x0020_0000), which then
            // hands control to the image vector table. The SoC sets VTOR to
            // that table before `reset()`; 0 is the architectural default.
            vtor: 0x0000_0000,
            nvic_enable: IrqMask::ZERO,
            nvic_pending: IrqMask::ZERO,
            irq_level: IrqMask::ZERO,
            nvic_prio: [0; NUM_IRQS],
            shpr: [0; 3],
            icsr_pendsv: false,
            icsr_pendst: false,
            wfi_hint: false,
            sysreset_request: false,
            syst_csr: 0,
            syst_rvr: 0,
            syst_cvr: 0,
            scr: 0,
            shcsr: 0,
            cpacr: 0,
            mpu_ctrl: 0,
            mpu_rnr: 0,
            mpu_rbar: 0,
            mpu_rlar: 0,
            mpu_mair: [0; 2],
            sau_ctrl: 0,
            sau_rnr: 0,
            sau_rbar: 0,
            sau_rlar: 0,
            fpregs: [0; 32],
            fpscr: 0,
            exc_return_pending: None,
            cycles: 0,
            break_cause: None,
        }
    }

    /// Architectural reset: load MSP and PC from the vector table at VTOR.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        let vtor = self.vtor;
        self.regs[SP] = self.read32(bus, vtor) & !0x3;
        self.regs[PC] = self.read32(bus, vtor.wrapping_add(4)) & !0x1;
        self.sp_sel = SpSel::Main;
        self.it_state = 0;
        self.ipsr = 0;
    }

    // --- xPSR pack/unpack (cold paths: MRS/MSR, exceptions) ---------------

    pub fn apsr(&self) -> u32 {
        (self.n as u32) << 31
            | (self.z as u32) << 30
            | (self.c as u32) << 29
            | (self.v as u32) << 28
            | (self.q as u32) << 27
            | (self.ge as u32) << 16
    }

    pub fn set_apsr(&mut self, value: u32) {
        self.n = value & (1 << 31) != 0;
        self.z = value & (1 << 30) != 0;
        self.c = value & (1 << 29) != 0;
        self.v = value & (1 << 28) != 0;
        self.q = value & (1 << 27) != 0;
        self.ge = ((value >> 16) & 0xf) as u8;
    }

    pub fn xpsr(&self) -> u32 {
        let it = self.it_state as u32;
        // EPSR: T bit 24 (always 1 here), IT[1:0] at 25:26, IT[7:2] at 10:15.
        self.apsr() | self.ipsr as u32 | 1 << 24 | (it & 0x3) << 25 | (it >> 2) << 10
    }

    // --- IT block state ----------------------------------------------------

    #[inline(always)]
    pub fn in_it_block(&self) -> bool {
        self.it_state & 0xf != 0
    }

    #[inline(always)]
    fn advance_it(&mut self) {
        if self.it_state & 0x7 == 0 {
            self.it_state = 0; // last instruction of the block
        } else {
            self.it_state = (self.it_state & 0xe0) | ((self.it_state << 1) & 0x1f);
        }
    }

    /// `ConditionPassed()` for condition code `cc` (0..=14).
    #[inline(always)]
    pub fn cond_passed(&self, cc: u32) -> bool {
        let base = match cc >> 1 {
            0b000 => self.z,
            0b001 => self.c,
            0b010 => self.n,
            0b011 => self.v,
            0b100 => self.c && !self.z,
            0b101 => self.n == self.v,
            0b110 => self.n == self.v && !self.z,
            _ => true, // AL (and 0b1111, which never reaches here)
        };
        if cc & 1 != 0 && cc != 0b1111 {
            !base
        } else {
            base
        }
    }

    // --- Memory access with PPB intercept ----------------------------------

    #[inline(always)]
    fn read32(&mut self, bus: &mut impl Bus, addr: u32) -> u32 {
        if addr >= 0xe000_0000 {
            self.ppb_read(addr)
        } else {
            bus.read32(addr)
        }
    }

    #[inline(always)]
    fn write32m(&mut self, bus: &mut impl Bus, addr: u32, value: u32) {
        if addr >= 0xe000_0000 {
            self.ppb_write(addr, value);
        } else {
            bus.write32(addr, value);
        }
    }

    #[inline(always)]
    fn read16m(&mut self, bus: &mut impl Bus, addr: u32) -> u16 {
        if addr >= 0xe000_0000 {
            (self.ppb_read(addr & !0x3) >> ((addr & 0x2) * 8)) as u16
        } else {
            bus.read16(addr)
        }
    }

    #[inline(always)]
    fn read8m(&mut self, bus: &mut impl Bus, addr: u32) -> u8 {
        if addr >= 0xe000_0000 {
            (self.ppb_read(addr & !0x3) >> ((addr & 0x3) * 8)) as u8
        } else {
            bus.read8(addr)
        }
    }

    #[inline(always)]
    fn write16m(&mut self, bus: &mut impl Bus, addr: u32, value: u16) {
        if addr >= 0xe000_0000 {
            // The M33 does true sub-word writes to the SCS: read-modify-
            // write the word.
            let word = self.ppb_read(addr & !0x3);
            let shift = (addr & 0x2) * 8;
            let merged = (word & !(0xffff << shift)) | (value as u32) << shift;
            self.ppb_write(addr & !0x3, merged);
        } else {
            bus.write16(addr, value);
        }
    }

    #[inline(always)]
    fn write8m(&mut self, bus: &mut impl Bus, addr: u32, value: u8) {
        if addr >= 0xe000_0000 {
            let word = self.ppb_read(addr & !0x3);
            let shift = (addr & 0x3) * 8;
            let merged = (word & !(0xff << shift)) | (value as u32) << shift;
            self.ppb_write(addr & !0x3, merged);
        } else {
            bus.write8(addr, value);
        }
    }

    /// PPB (System Control Space) reads: SysTick, NVIC, SCB, MPU, SAU.
    fn ppb_read(&mut self, addr: u32) -> u32 {
        match addr {
            // --- SysTick ---
            0xe000_e010 => {
                let v = self.syst_csr;
                self.syst_csr &= !(1 << 16); // COUNTFLAG clears on read
                v
            }
            0xe000_e014 => self.syst_rvr,
            0xe000_e018 => self.syst_cvr,
            0xe000_e01c => 0, // SYST_CALIB: no reference info
            // --- NVIC (158 IRQs: five words per bank) ---
            0xe000_e100..=0xe000_e110 => {
                self.nvic_enable.word(nvic_word_index(addr)) // ISER0-4
            }
            0xe000_e180..=0xe000_e190 => {
                self.nvic_enable.word(nvic_word_index(addr)) // ICER reads like ISER
            }
            0xe000_e200..=0xe000_e210 => {
                self.nvic_pending
                    .or(&self.irq_level)
                    .word(nvic_word_index(addr)) // ISPR0-4
            }
            0xe000_e280..=0xe000_e290 => {
                self.nvic_pending
                    .or(&self.irq_level)
                    .word(nvic_word_index(addr)) // ICPR0-4
            }
            0xe000_e400..=0xe000_e4a0 => {
                // IPR0-18: one byte per IRQ, packed 4 per word.
                let base = ((addr - 0xe000_e400) as usize) & !0x3;
                let mut word = 0u32;
                for i in 0..4 {
                    if base + i < NUM_IRQS {
                        word |= (self.nvic_prio[base + i] as u32) << (8 * i);
                    }
                }
                word
            }
            // --- SCB ---
            0xe000_ed00 => 0x411f_c272, // CPUID: Cortex-M7 r1p2 (i.MX RT1060)
            0xe000_ed04 => {
                // ICSR: VECTACTIVE | PENDSTSET(26) | PENDSVSET(28).
                (self.ipsr as u32)
                    | (self.icsr_pendst as u32) << 26
                    | (self.icsr_pendsv as u32) << 28
            }
            0xe000_ed08 => self.vtor,
            0xe000_ed0c => 0xfa05_0000, // AIRCR: VECTKEYSTAT
            0xe000_ed10 => self.scr,
            0xe000_ed14 => 0x0000_0201, // CCR: STKALIGN | USERSETMPEND-ish defaults
            0xe000_ed18 => self.shpr[0],
            0xe000_ed1c => self.shpr[1],
            0xe000_ed20 => self.shpr[2],
            0xe000_ed24 => self.shcsr,
            0xe000_ed88 => self.cpacr,
            // --- MPU (stored-readback; no enforcement) ---
            // EFR32MG24: sl_mpu_disable_execute_from_ram runs at boot and
            // programs these — it only needs writes to stick and
            // MPU_TYPE.DREGION != 0.
            0xe000_ed90 => 0x0000_1000, // MPU_TYPE: 16 data regions (M7 PMSAv7)
            0xe000_ed94 => self.mpu_ctrl,
            0xe000_ed98 => self.mpu_rnr,
            0xe000_ed9c => self.mpu_rbar,
            0xe000_eda0 => self.mpu_rlar,
            0xe000_edc0 => self.mpu_mair[0],
            0xe000_edc4 => self.mpu_mair[1],
            // --- SAU (stored-readback; TrustZone-lite, docs/DESIGN.md §2) ---
            0xe000_edd0 => self.sau_ctrl,
            0xe000_edd4 => 0x8, // SAU_TYPE: 8 regions
            0xe000_edd8 => self.sau_rnr,
            0xe000_eddc => self.sau_rbar,
            0xe000_ede0 => self.sau_rlar,
            _ => 0,
        }
    }

    /// PPB writes: SysTick, NVIC, SCB, MPU, SAU.
    fn ppb_write(&mut self, addr: u32, value: u32) {
        match addr {
            // --- SysTick ---
            0xe000_e010 => self.syst_csr = (self.syst_csr & 1 << 16) | (value & 0x7),
            0xe000_e014 => self.syst_rvr = value & 0x00ff_ffff,
            0xe000_e018 => {
                // Any write clears CVR and COUNTFLAG.
                self.syst_cvr = 0;
                self.syst_csr &= !(1 << 16);
            }
            // --- NVIC (bits for nonexistent IRQs are RAZ/WI) ---
            0xe000_e100..=0xe000_e110 => {
                self.nvic_enable.set_word(nvic_word_index(addr), value); // ISER
            }
            0xe000_e180..=0xe000_e190 => {
                self.nvic_enable.clear_word(nvic_word_index(addr), value); // ICER
            }
            0xe000_e200..=0xe000_e210 => {
                self.nvic_pending.set_word(nvic_word_index(addr), value); // ISPR
            }
            0xe000_e280..=0xe000_e290 => {
                self.nvic_pending.clear_word(nvic_word_index(addr), value); // ICPR
            }
            0xe000_e400..=0xe000_e4a0 => {
                let base = ((addr - 0xe000_e400) as usize) & !0x3;
                for i in 0..4 {
                    if base + i < NUM_IRQS {
                        // The MG24's M33 implements 4 priority bits
                        // (top nibble), same as the RP2350.
                        self.nvic_prio[base + i] = ((value >> (8 * i)) & 0xf0) as u8;
                    }
                }
            }
            // --- SCB ---
            0xe000_ed04 => {
                // ICSR: PENDSVSET(28)/PENDSVCLR(27)/PENDSTSET(26)/PENDSTCLR(25).
                if value & 1 << 28 != 0 {
                    self.icsr_pendsv = true;
                }
                if value & 1 << 27 != 0 {
                    self.icsr_pendsv = false;
                }
                if value & 1 << 26 != 0 {
                    self.icsr_pendst = true;
                }
                if value & 1 << 25 != 0 {
                    self.icsr_pendst = false;
                }
            }
            0xe000_ed08 => {
                // Armv8-M reserves VTOR bits [6:0]; software must align the
                // table further for the 92 vectors (16 system + 76 IRQs).
                self.vtor = value & 0xffff_ff80;
            }
            0xe000_ed0c => {
                // AIRCR: VECTKEY-gated. SYSRESETREQ (bit 2) requests a
                // system reset the chip runner performs — on the EFR32 it
                // latches EMU RSTCAUSE.SYSREQ (0x40), as observed on real
                // hardware via openocd `reset run`
                // (tests/fixtures/mg24_m7_wdog.serial-reference.txt:
                // "cause 0x40 wdog=0").
                if value >> 16 == 0x05fa && value & 0x4 != 0 {
                    self.sysreset_request = true;
                }
            }
            0xe000_ed10 => self.scr = value & 0x16,
            0xe000_ed18 => self.shpr[0] = value & 0xf0f0_f0f0,
            0xe000_ed1c => self.shpr[1] = value & 0xf0f0_f0f0,
            0xe000_ed20 => self.shpr[2] = value & 0xf0f0_f0f0,
            0xe000_ed24 => self.shcsr = value,
            0xe000_ed88 => self.cpacr = value,
            // --- MPU (stored-readback; no enforcement) ---
            0xe000_ed94 => self.mpu_ctrl = value,
            0xe000_ed98 => self.mpu_rnr = value,
            0xe000_ed9c => self.mpu_rbar = value,
            0xe000_eda0 => self.mpu_rlar = value,
            0xe000_edc0 => self.mpu_mair[0] = value,
            0xe000_edc4 => self.mpu_mair[1] = value,
            // --- SAU (stored-readback; TrustZone-lite, docs/DESIGN.md §2) ---
            0xe000_edd0 => self.sau_ctrl = value,
            0xe000_edd8 => self.sau_rnr = value,
            0xe000_eddc => self.sau_rbar = value,
            0xe000_ede0 => self.sau_rlar = value,
            _ => {}
        }
    }

    // --- Exceptions ---------------------------------------------------------

    /// Priority of exception number `n` (16+ are external IRQs). Lower is
    /// more urgent; NMI/HardFault are fixed at -2/-1.
    fn exc_prio(&self, n: u16) -> i16 {
        match n {
            2 => -2,
            3 => -1,
            // SHPR1-3 hold handlers 4-15, one byte each.
            4..=15 => {
                ((self.shpr[(n as usize - 4) / 4] >> (8 * ((n as usize - 4) % 4))) & 0xff) as i16
            }
            _ => self.nvic_prio[n as usize - 16] as i16,
        }
    }

    /// Current execution priority (Armv8-M `ExecutionPriority()`, less
    /// PRIGROUP subtleties — MG24 firmware doesn't split groups).
    fn execution_priority(&self) -> i16 {
        let mut p = 256;
        if self.ipsr != 0 {
            p = self.exc_prio(self.ipsr);
        }
        if self.basepri != 0 {
            p = p.min((self.basepri & 0xf0) as i16);
        }
        if self.primask {
            p = p.min(0);
        }
        if self.faultmask {
            p = p.min(-1);
        }
        p
    }

    /// Take the highest-priority pending exception if it preempts.
    #[cold]
    fn take_pending_exception(&mut self, bus: &mut impl Bus) {
        let mut best: (i16, u16) = (256, 0);
        if self.icsr_pendst {
            let p = self.exc_prio(15);
            if p < best.0 {
                best = (p, 15);
            }
        }
        if self.icsr_pendsv {
            let p = self.exc_prio(14);
            if p < best.0 {
                best = (p, 14);
            }
        }
        let mut ready = self.nvic_pending.or(&self.irq_level).and(&self.nvic_enable);
        while let Some(irq) = ready.lowest_set() {
            ready.clear(irq);
            let irq = irq as u16;
            let p = self.exc_prio(16 + irq);
            // Tie goes to the lowest exception number.
            if p < best.0 || (p == best.0 && best.1 != 0 && 16 + irq < best.1) {
                best = (p, 16 + irq);
            }
        }
        if best.1 != 0 && best.0 < self.execution_priority() {
            self.exception_entry(bus, best.1);
        }
    }

    /// Armv8-M exception entry: push the stack frame, switch to MSP,
    /// load the handler address from the vector table.
    fn exception_entry(&mut self, bus: &mut impl Bus, n: u16) {
        // Clear the pending source we are about to service.
        match n {
            14 => self.icsr_pendsv = false,
            15 => self.icsr_pendst = false,
            _ if n >= 16 => self.nvic_pending.clear((n - 16) as u32),
            _ => {}
        }
        let return_addr = self.regs[PC];
        let mut sp = self.regs[SP];
        let pad = sp & 0x4 != 0;
        sp = (sp - 0x20) & !0x7;
        self.write32m(bus, sp, self.regs[0]);
        self.write32m(bus, sp + 4, self.regs[1]);
        self.write32m(bus, sp + 8, self.regs[2]);
        self.write32m(bus, sp + 12, self.regs[3]);
        self.write32m(bus, sp + 16, self.regs[12]);
        self.write32m(bus, sp + 20, self.regs[LR]);
        self.write32m(bus, sp + 24, return_addr);
        self.write32m(bus, sp + 28, self.xpsr() | (pad as u32) << 9);
        self.regs[SP] = sp;
        // EXC_RETURN: bit3 = came from thread, bit2 = came from PSP;
        // 0xFFFFFFE0 base | ES(bit0)=1 (Secure) | DCRS(bit5)=1 | FType(bit4)=1.
        let from_thread = self.ipsr == 0;
        let from_psp = self.sp_sel == SpSel::Process;
        self.regs[LR] = 0xffff_ffe1
            | 1 << 5
            | 1 << 4
            | (from_thread as u32) << 3
            | ((from_thread && from_psp) as u32) << 2;
        self.switch_sp(SpSel::Main);
        self.ipsr = n;
        self.it_state = 0;
        let vector = self.read32(bus, self.vtor.wrapping_add(4 * n as u32));
        self.regs[PC] = vector & !0x1;
    }

    /// Armv8-M exception return (branch to an 0xFFxxxxxx EXC_RETURN value):
    /// pop the frame from the selected stack and restore state.
    #[cold]
    fn exception_return(&mut self, bus: &mut impl Bus, exc_ret: u32) {
        let to_thread = exc_ret & 0x8 != 0;
        let use_psp = to_thread && exc_ret & 0x4 != 0;
        self.switch_sp(if use_psp { SpSel::Process } else { SpSel::Main });
        let sp = self.regs[SP];
        self.regs[0] = self.read32(bus, sp);
        self.regs[1] = self.read32(bus, sp + 4);
        self.regs[2] = self.read32(bus, sp + 8);
        self.regs[3] = self.read32(bus, sp + 12);
        self.regs[12] = self.read32(bus, sp + 16);
        self.regs[LR] = self.read32(bus, sp + 20);
        let return_addr = self.read32(bus, sp + 24);
        let xpsr = self.read32(bus, sp + 28);
        self.regs[SP] = sp + 0x20 + ((xpsr >> 9 & 1) << 2);
        self.regs[PC] = return_addr & !0x1;
        self.set_apsr(xpsr);
        self.ipsr = if to_thread { 0 } else { (xpsr & 0x1ff) as u16 };
        self.it_state = (((xpsr >> 25) & 0x3) | ((xpsr >> 10 & 0x3f) << 2)) as u8;
    }

    // --- Flag helpers -------------------------------------------------------

    #[inline(always)]
    fn set_nz(&mut self, result: u32) {
        self.n = result & 0x8000_0000 != 0;
        self.z = result == 0;
    }

    #[inline(always)]
    fn add_update(&mut self, x: u32, y: u32, carry_in: bool, setflags: bool) -> u32 {
        let (result, c, v) = add_with_carry(x, y, carry_in);
        if setflags {
            self.set_nz(result);
            self.c = c;
            self.v = v;
        }
        result
    }

    // Shift helpers implementing the Arm ARM `Shift_C` semantics for
    // register-controlled shifts (amount already masked to 0..=255).

    fn lsl_c(&mut self, x: u32, amount: u32, setflags: bool) -> u32 {
        let result = if amount == 0 {
            x
        } else if amount < 32 {
            if setflags {
                self.c = (x >> (32 - amount)) & 1 != 0;
            }
            x << amount
        } else {
            if setflags {
                self.c = amount == 32 && x & 1 != 0;
            }
            0
        };
        if setflags {
            self.set_nz(result);
        }
        result
    }

    fn lsr_c(&mut self, x: u32, amount: u32, setflags: bool) -> u32 {
        let result = if amount == 0 {
            x
        } else if amount < 32 {
            if setflags {
                self.c = (x >> (amount - 1)) & 1 != 0;
            }
            x >> amount
        } else {
            if setflags {
                self.c = amount == 32 && x & 0x8000_0000 != 0;
            }
            0
        };
        if setflags {
            self.set_nz(result);
        }
        result
    }

    fn asr_c(&mut self, x: u32, amount: u32, setflags: bool) -> u32 {
        let result = if amount == 0 {
            x
        } else if amount < 32 {
            if setflags {
                self.c = (x >> (amount - 1)) & 1 != 0;
            }
            ((x as i32) >> amount) as u32
        } else {
            if setflags {
                self.c = x & 0x8000_0000 != 0;
            }
            ((x as i32) >> 31) as u32
        };
        if setflags {
            self.set_nz(result);
        }
        result
    }

    fn ror_c(&mut self, x: u32, amount: u32, setflags: bool) -> u32 {
        let result = if amount == 0 {
            x
        } else {
            let r = x.rotate_right(amount & 31);
            if setflags {
                self.c = r & 0x8000_0000 != 0;
            }
            r
        };
        if setflags {
            self.set_nz(result);
        }
        result
    }

    // --- Branching ----------------------------------------------------------

    /// Write PC from a BX/BLX/POP/LDM-style interworking address. In
    /// handler mode, 0xFFxxxxxx values are EXC_RETURN magic and perform an
    /// exception return; the pending flag defers it to the step loop, which
    /// has bus access.
    #[inline(always)]
    fn branch_interworking(&mut self, target: u32) {
        // T bit must be 1 in M-profile; a 0 here faults on hardware. We
        // clear it and keep going — firmware that does this is already lost.
        self.regs[PC] = target & !0x1;
        if target >= 0xff00_0000 && self.ipsr != 0 {
            self.exc_return_pending = Some(target);
        }
    }

    // --- Execution ----------------------------------------------------------

    /// Execute one instruction. Returns the number of cycles consumed
    /// (currently 1 per instruction; timing model refined later).
    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        // A branch to EXC_RETURN magic completes here, where the bus is
        // available (possibly chaining straight into another exception).
        if let Some(exc_ret) = self.exc_return_pending.take() {
            self.exception_return(bus, exc_ret);
        }
        // SysTick, processor-clock source (CSR.CLKSOURCE==1): one tick per
        // instruction (cycle model is 1 IPC). The external reference source
        // (CLKSOURCE==0) is fixed in wall-clock time, so the SoC drives it from
        // retired cycles instead (`Rt1060::tick_systick_external`).
        if self.syst_csr & 0b101 == 0b101 {
            if self.syst_cvr == 0 {
                self.syst_cvr = self.syst_rvr & 0x00ff_ffff;
            } else {
                self.syst_cvr -= 1;
                if self.syst_cvr == 0 {
                    self.syst_csr |= 1 << 16; // COUNTFLAG
                    if self.syst_csr & 0x2 != 0 {
                        self.icsr_pendst = true;
                    }
                }
            }
        }
        // Sample level-sensitive IRQ lines (not latched: a line that drops
        // before being taken stops being pending, per Armv8-M).
        self.irq_level = bus.irq_lines();
        // Preempt if a pending exception outranks the current priority.
        if self.icsr_pendst
            || self.icsr_pendsv
            || !self
                .nvic_pending
                .or(&self.irq_level)
                .and(&self.nvic_enable)
                .is_zero()
        {
            self.take_pending_exception(bus);
        }
        let pc = self.regs[PC];

        // Fast path: pre-decoded ops (cached for flash by the bus). The
        // IT-block path stays raw — IT semantics change flag behavior, so
        // decoded ops (which bake in flag-setting) would be wrong there.
        // PPB fetches stay raw too: read16m intercepts that region.
        if !self.in_it_block() && pc < 0xe000_0000 {
            let op = bus.fetch_op(pc);
            self.exec_decoded(bus, op, pc);
            self.cycles += 1;
            return 1;
        }

        let hw1 = self.read16m(bus, pc);
        let wide = hw1 >= 0xe800;
        // Value the instruction sees when it reads PC (Arm ARM: current + 4).
        let pc4 = pc.wrapping_add(4);
        self.regs[PC] = pc.wrapping_add(if wide { 4 } else { 2 });

        // IT block: check the current slot's condition, then advance state.
        if self.in_it_block() {
            let cond = (self.it_state >> 4) as u32;
            let in_it = true;
            self.advance_it();
            if !self.cond_passed(cond) {
                self.cycles += 1;
                return 1;
            }
            if wide {
                let hw2 = self.read16m(bus, pc.wrapping_add(2));
                self.exec_wide(bus, hw1, hw2, pc4, in_it);
            } else {
                self.exec_narrow(bus, hw1, pc4, in_it);
            }
        } else if wide {
            let hw2 = self.read16m(bus, pc.wrapping_add(2));
            self.exec_wide(bus, hw1, hw2, pc4, false);
        } else {
            self.exec_narrow(bus, hw1, pc4, false);
        }
        self.cycles += 1;
        1
    }

    /// 16-bit Thumb instructions (complete Armv8-M baseline set).
    fn exec_narrow(&mut self, bus: &mut impl Bus, hw: u16, pc4: u32, in_it: bool) {
        let op = hw as u32;
        // Data-processing narrow encodings set flags only outside IT blocks.
        let sf = !in_it;
        match op >> 12 {
            0x0 | 0x1 => {
                let rd = (op & 0x7) as usize;
                let rn = ((op >> 3) & 0x7) as usize;
                match (op >> 11) & 0x3 {
                    0b00 => {
                        // MOVS (imm5==0) / LSLS: imm5==0 leaves C unchanged.
                        let imm5 = (op >> 6) & 0x1f;
                        let x = self.regs[rn];
                        let result = if imm5 == 0 {
                            x
                        } else {
                            self.lsl_c(x, imm5, sf)
                        };
                        if imm5 == 0 && sf {
                            self.set_nz(result);
                        }
                        self.regs[rd] = result;
                    }
                    0b01 => {
                        // LSRS: imm5==0 means shift 32.
                        let imm5 = (op >> 6) & 0x1f;
                        let amount = if imm5 == 0 { 32 } else { imm5 };
                        self.regs[rd] = self.lsr_c(self.regs[rn], amount, sf);
                    }
                    0b10 => {
                        // ASRS: imm5==0 means shift 32.
                        let imm5 = (op >> 6) & 0x1f;
                        let amount = if imm5 == 0 { 32 } else { imm5 };
                        self.regs[rd] = self.asr_c(self.regs[rn], amount, sf);
                    }
                    _ => {
                        let x = self.regs[rn];
                        let rm_or_imm = (op >> 6) & 0x7;
                        match (op >> 9) & 0x3 {
                            0b00 => {
                                self.regs[rd] =
                                    self.add_update(x, self.regs[rm_or_imm as usize], false, sf)
                            }
                            0b01 => {
                                self.regs[rd] =
                                    self.add_update(x, !self.regs[rm_or_imm as usize], true, sf)
                            }
                            0b10 => self.regs[rd] = self.add_update(x, rm_or_imm, false, sf),
                            _ => self.regs[rd] = self.add_update(x, !rm_or_imm, true, sf),
                        }
                    }
                }
            }
            0x2 | 0x3 => {
                let rd = ((op >> 8) & 0x7) as usize;
                let imm8 = op & 0xff;
                match (op >> 11) & 0x3 {
                    0b00 => {
                        self.regs[rd] = imm8;
                        if sf {
                            self.set_nz(imm8);
                        }
                    }
                    // CMP always sets flags, IT block or not.
                    0b01 => {
                        self.add_update(self.regs[rd], !imm8, true, true);
                    }
                    0b10 => self.regs[rd] = self.add_update(self.regs[rd], imm8, false, sf),
                    _ => self.regs[rd] = self.add_update(self.regs[rd], !imm8, true, sf),
                }
            }
            0x4 => match (op >> 10) & 0x3 {
                0b00 => {
                    let rd = (op & 0x7) as usize;
                    let rm = ((op >> 3) & 0x7) as usize;
                    let x = self.regs[rd];
                    let y = self.regs[rm];
                    match (op >> 6) & 0xf {
                        0x0 => {
                            self.regs[rd] = x & y;
                            if sf {
                                self.set_nz(x & y);
                            }
                        }
                        0x1 => {
                            self.regs[rd] = x ^ y;
                            if sf {
                                self.set_nz(x ^ y);
                            }
                        }
                        0x2 => self.regs[rd] = self.lsl_c(x, y & 0xff, sf),
                        0x3 => self.regs[rd] = self.lsr_c(x, y & 0xff, sf),
                        0x4 => self.regs[rd] = self.asr_c(x, y & 0xff, sf),
                        0x5 => {
                            let c = self.c;
                            self.regs[rd] = self.add_update(x, y, c, sf);
                        }
                        0x6 => {
                            let c = self.c;
                            self.regs[rd] = self.add_update(x, !y, c, sf);
                        }
                        0x7 => {
                            let amount = y & 0xff;
                            let r = if amount == 0 {
                                if sf {
                                    self.set_nz(x);
                                }
                                x
                            } else {
                                self.ror_c(x, amount, sf)
                            };
                            self.regs[rd] = r;
                        }
                        // TST/CMP/CMN always set flags.
                        0x8 => self.set_nz(x & y),
                        0x9 => self.regs[rd] = self.add_update(!y, 0, true, sf), // RSBS Rd, Rm, #0
                        0xa => {
                            self.add_update(x, !y, true, true);
                        }
                        0xb => {
                            self.add_update(x, y, false, true);
                        }
                        0xc => {
                            self.regs[rd] = x | y;
                            if sf {
                                self.set_nz(x | y);
                            }
                        }
                        0xd => {
                            // MULS: N and Z only; C, V unaffected.
                            let r = x.wrapping_mul(y);
                            self.regs[rd] = r;
                            if sf {
                                self.set_nz(r);
                            }
                        }
                        0xe => {
                            self.regs[rd] = x & !y;
                            if sf {
                                self.set_nz(x & !y);
                            }
                        }
                        _ => {
                            self.regs[rd] = !y;
                            if sf {
                                self.set_nz(!y);
                            }
                        }
                    }
                }
                0b01 => {
                    // Special data + BX/BLX (hi registers, no flags).
                    let rm = ((op >> 3) & 0xf) as usize;
                    let rd = ((op & 0x7) | ((op >> 4) & 0x8)) as usize;
                    let rm_val = if rm == PC { pc4 } else { self.regs[rm] };
                    match (op >> 8) & 0x3 {
                        0b00 => {
                            let rd_val = if rd == PC { pc4 } else { self.regs[rd] };
                            let result = rd_val.wrapping_add(rm_val);
                            if rd == PC {
                                self.regs[PC] = result & !0x1;
                            } else {
                                self.regs[rd] = result;
                            }
                        }
                        0b01 => {
                            let rd_val = if rd == PC { pc4 } else { self.regs[rd] };
                            self.add_update(rd_val, !rm_val, true, true);
                        }
                        0b10 => {
                            if rd == PC {
                                self.regs[PC] = rm_val & !0x1;
                            } else {
                                self.regs[rd] = rm_val;
                            }
                        }
                        _ => {
                            if op & 0x80 != 0 {
                                // BLX Rm: return address is the next instruction.
                                let ret = self.regs[PC] | 1;
                                self.regs[LR] = ret;
                            }
                            self.branch_interworking(rm_val);
                        }
                    }
                }
                _ => {
                    // LDR (literal): word-aligned PC.
                    let rt = ((op >> 8) & 0x7) as usize;
                    let addr = (pc4 & !0x3).wrapping_add((op & 0xff) << 2);
                    self.regs[rt] = self.read32(bus, addr);
                }
            },
            0x5 => {
                let rm = ((op >> 6) & 0x7) as usize;
                let rn = ((op >> 3) & 0x7) as usize;
                let rt = (op & 0x7) as usize;
                let addr = self.regs[rn].wrapping_add(self.regs[rm]);
                match (op >> 9) & 0x7 {
                    0b000 => self.write32m(bus, addr, self.regs[rt]),
                    0b001 => self.write16m(bus, addr, self.regs[rt] as u16),
                    0b010 => self.write8m(bus, addr, self.regs[rt] as u8),
                    0b011 => self.regs[rt] = self.read8m(bus, addr) as i8 as i32 as u32,
                    0b100 => self.regs[rt] = self.read32(bus, addr),
                    0b101 => self.regs[rt] = self.read16m(bus, addr) as u32,
                    0b110 => self.regs[rt] = self.read8m(bus, addr) as u32,
                    _ => self.regs[rt] = self.read16m(bus, addr) as i16 as i32 as u32,
                }
            }
            0x6 => {
                // STR/LDR immediate (imm5 * 4).
                let imm = ((op >> 6) & 0x1f) << 2;
                let rn = ((op >> 3) & 0x7) as usize;
                let rt = (op & 0x7) as usize;
                let addr = self.regs[rn].wrapping_add(imm);
                if op & 0x800 != 0 {
                    self.regs[rt] = self.read32(bus, addr);
                } else {
                    self.write32m(bus, addr, self.regs[rt]);
                }
            }
            0x7 => {
                // STRB/LDRB immediate.
                let imm = (op >> 6) & 0x1f;
                let rn = ((op >> 3) & 0x7) as usize;
                let rt = (op & 0x7) as usize;
                let addr = self.regs[rn].wrapping_add(imm);
                if op & 0x800 != 0 {
                    self.regs[rt] = self.read8m(bus, addr) as u32;
                } else {
                    self.write8m(bus, addr, self.regs[rt] as u8);
                }
            }
            0x8 => {
                // STRH/LDRH immediate (imm5 * 2).
                let imm = ((op >> 6) & 0x1f) << 1;
                let rn = ((op >> 3) & 0x7) as usize;
                let rt = (op & 0x7) as usize;
                let addr = self.regs[rn].wrapping_add(imm);
                if op & 0x800 != 0 {
                    self.regs[rt] = self.read16m(bus, addr) as u32;
                } else {
                    self.write16m(bus, addr, self.regs[rt] as u16);
                }
            }
            0x9 => {
                // STR/LDR SP-relative (imm8 * 4).
                let rt = ((op >> 8) & 0x7) as usize;
                let addr = self.regs[SP].wrapping_add((op & 0xff) << 2);
                if op & 0x800 != 0 {
                    self.regs[rt] = self.read32(bus, addr);
                } else {
                    self.write32m(bus, addr, self.regs[rt]);
                }
            }
            0xa => {
                // ADR / ADD Rd, SP, #imm8*4.
                let rd = ((op >> 8) & 0x7) as usize;
                let imm = (op & 0xff) << 2;
                self.regs[rd] = if op & 0x800 != 0 {
                    self.regs[SP].wrapping_add(imm)
                } else {
                    (pc4 & !0x3).wrapping_add(imm)
                };
            }
            0xb => self.exec_misc(bus, op, in_it),
            0xc => {
                // STM/LDM (always writeback unless Rn in LDM list).
                let rn = ((op >> 8) & 0x7) as usize;
                let list = op & 0xff;
                let mut addr = self.regs[rn];
                if op & 0x800 != 0 {
                    for i in 0..8 {
                        if list & (1 << i) != 0 {
                            self.regs[i] = self.read32(bus, addr);
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if list & (1 << rn) == 0 {
                        self.regs[rn] = addr;
                    }
                } else {
                    for i in 0..8 {
                        if list & (1 << i) != 0 {
                            self.write32m(bus, addr, self.regs[i]);
                            addr = addr.wrapping_add(4);
                        }
                    }
                    self.regs[rn] = addr;
                }
            }
            0xd => {
                let cond = (op >> 8) & 0xf;
                match cond {
                    0xe => self.break_cause = Some(BreakCause::Udf((op & 0xff) as u8)),
                    0xf => self.exception_entry(bus, 11), // SVCall
                    _ => {
                        if self.cond_passed(cond) {
                            let imm = ((op & 0xff) as i8 as i32) << 1;
                            self.regs[PC] = pc4.wrapping_add(imm as u32);
                        }
                    }
                }
            }
            _ => {
                // 0xE000..0xE7FF: unconditional branch imm11.
                let imm = ((op & 0x7ff) << 21) as i32 >> 20;
                self.regs[PC] = pc4.wrapping_add(imm as u32);
            }
        }
    }

    /// Miscellaneous 16-bit instructions (0xBxxx block).
    fn exec_misc(&mut self, bus: &mut impl Bus, op: u32, in_it: bool) {
        match (op >> 8) & 0xf {
            0x0 => {
                // ADD/SUB SP, #imm7*4.
                let imm = (op & 0x7f) << 2;
                if op & 0x80 != 0 {
                    self.regs[SP] = self.regs[SP].wrapping_sub(imm);
                } else {
                    self.regs[SP] = self.regs[SP].wrapping_add(imm);
                }
            }
            0x1 | 0x3 | 0x9 | 0xb => {
                // CBZ/CBNZ (never in IT block; compare-only, no flags).
                let rn = (op & 0x7) as usize;
                let imm = ((op >> 3) & 0x1f) << 1 | ((op >> 9) & 0x1) << 6;
                let nonzero = op & 0x800 != 0;
                if (self.regs[rn] == 0) != nonzero {
                    // regs[PC] currently holds next-instruction address =
                    // pc+2; branch target is pc4 + imm = regs[PC] + 2 + imm.
                    self.regs[PC] = self.regs[PC].wrapping_add(2).wrapping_add(imm);
                }
            }
            0x2 => {
                let rd = (op & 0x7) as usize;
                let rm = ((op >> 3) & 0x7) as usize;
                let x = self.regs[rm];
                self.regs[rd] = match (op >> 6) & 0x3 {
                    0b00 => x as i16 as i32 as u32,
                    0b01 => x as i8 as i32 as u32,
                    0b10 => x & 0xffff,
                    _ => x & 0xff,
                };
            }
            0x4 | 0x5 => {
                // PUSH: lowest register at lowest address.
                let list = op & 0xff;
                let lr_bit = op & 0x100 != 0;
                let count = (list.count_ones() + lr_bit as u32) * 4;
                let mut addr = self.regs[SP].wrapping_sub(count);
                self.regs[SP] = addr;
                for i in 0..8 {
                    if list & (1 << i) != 0 {
                        self.write32m(bus, addr, self.regs[i]);
                        addr = addr.wrapping_add(4);
                    }
                }
                if lr_bit {
                    self.write32m(bus, addr, self.regs[LR]);
                }
            }
            0x6 => {
                // CPS: privilege check deferred (we run privileged).
                if op & 0xef == 0x62 {
                    self.primask = op & 0x10 != 0; // CPSID i / CPSIE i
                }
            }
            0xa => {
                let rd = (op & 0x7) as usize;
                let rm = ((op >> 3) & 0x7) as usize;
                let x = self.regs[rm];
                self.regs[rd] = match (op >> 6) & 0x3 {
                    0b00 => x.swap_bytes(),
                    0b01 => (x & 0xff00_ff00) >> 8 | (x & 0x00ff_00ff) << 8,
                    0b11 => (x as u16).swap_bytes() as i16 as i32 as u32,
                    _ => x, // 0b10: HLT (Armv8-M debug) — treat as nop
                };
            }
            0xc | 0xd => {
                // POP: PC bit performs an interworking branch.
                let list = op & 0xff;
                let mut addr = self.regs[SP];
                for i in 0..8 {
                    if list & (1 << i) != 0 {
                        self.regs[i] = self.read32(bus, addr);
                        addr = addr.wrapping_add(4);
                    }
                }
                if op & 0x100 != 0 {
                    let target = self.read32(bus, addr);
                    addr = addr.wrapping_add(4);
                    self.regs[SP] = addr;
                    self.branch_interworking(target);
                } else {
                    self.regs[SP] = addr;
                }
            }
            0xe => self.break_cause = Some(BreakCause::Bkpt((op & 0xff) as u8)),
            0xf => {
                let mask = op & 0xf;
                if mask != 0 && !in_it {
                    // IT: ITSTATE = firstcond:mask.
                    self.it_state = (op & 0xff) as u8;
                }
                // else hints. WFI (0xBF30) / WFE (0xBF20) signal the run
                // loop so it can fast-forward idle time to the next timer
                // deadline (EM1 sleep; docs/DESIGN.md time model). NOP/SEV/
                // YIELD: nothing to do (single core yet).
                if mask == 0 && matches!((op >> 4) & 0xf, 0x2 | 0x3) {
                    self.wfi_hint = true;
                }
            }
            _ => self.break_cause = Some(BreakCause::Unimplemented(op)),
        }
    }

    /// Advance SysTick by a batch of cycles at once (WFI fast-forward).
    /// Mirrors the per-step logic in `step()`: cvr==0 reloads without
    /// decrementing, so the fire period is RVR+1 steps.
    pub fn systick_advance(&mut self, cycles: u32) {
        if self.syst_csr & 0x1 == 0 || cycles == 0 {
            return;
        }
        let reload = self.syst_rvr & 0x00ff_ffff;
        let mut n = cycles as u64;
        // Account for a reload step if we start at zero.
        if self.syst_cvr == 0 {
            if reload == 0 {
                return; // degenerate: reloads to 0 forever, never fires
            }
            self.syst_cvr = reload;
            n -= 1;
        }
        if n < self.syst_cvr as u64 {
            self.syst_cvr -= n as u32;
            return;
        }
        // At least one fire.
        n -= self.syst_cvr as u64;
        self.syst_csr |= 1 << 16; // COUNTFLAG
        if self.syst_csr & 0x2 != 0 {
            self.icsr_pendst = true;
        }
        let period = reload as u64 + 1;
        if reload == 0 {
            self.syst_cvr = 0;
            return;
        }
        let rem = n % period;
        // rem steps past a fire: first step reloads, rest decrement.
        self.syst_cvr = if rem == 0 {
            0
        } else {
            reload - (rem as u32 - 1)
        };
        if n >= period && self.syst_csr & 0x2 != 0 {
            self.icsr_pendst = true; // additional wraps still just pend
        }
    }

    // --- Banked stack pointers ----------------------------------------------

    pub fn msp(&self) -> u32 {
        match self.sp_sel {
            SpSel::Main => self.regs[SP],
            SpSel::Process => self.sp_bank,
        }
    }

    pub fn psp(&self) -> u32 {
        match self.sp_sel {
            SpSel::Main => self.sp_bank,
            SpSel::Process => self.regs[SP],
        }
    }

    pub fn set_msp(&mut self, value: u32) {
        match self.sp_sel {
            SpSel::Main => self.regs[SP] = value,
            SpSel::Process => self.sp_bank = value,
        }
    }

    pub fn set_psp(&mut self, value: u32) {
        match self.sp_sel {
            SpSel::Main => self.sp_bank = value,
            SpSel::Process => self.regs[SP] = value,
        }
    }

    fn switch_sp(&mut self, want: SpSel) {
        if want != self.sp_sel {
            std::mem::swap(&mut self.regs[SP], &mut self.sp_bank);
            self.sp_sel = want;
        }
    }
}

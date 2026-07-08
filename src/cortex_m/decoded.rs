//! Decode-once instruction cache (the M7 design from docs/DESIGN.md).
//!
//! [`DecodedOp`] is an 8-byte `Copy` value: the hot instructions from the
//! real-firmware histogram carry pre-extracted operands and pre-computed
//! absolute branch targets; everything else falls back to the raw
//! halfwords and the existing executors, so instruction *semantics* live
//! in exactly one place. The `SystemBus` caches decoded slots for flash —
//! where real firmware executes — and invalidates them on the few
//! paths that can mutate flash. SRAM execution decodes on the fly.
//!
//! IT blocks bypass the cache (their members must not set flags and are
//! rare); the core keeps its original path for them.

use super::{Bus, CortexM7, LR, PC, SP};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodedOp {
    /// Cache slot not yet decoded (or invalidated).
    NotDecoded,
    // --- ALU, immediate ---
    MovsImm8 {
        rd: u8,
        imm: u8,
    },
    CmpImm8 {
        rn: u8,
        imm: u8,
    },
    AddsImm8 {
        rdn: u8,
        imm: u8,
    },
    SubsImm8 {
        rdn: u8,
        imm: u8,
    },
    AddsImm3 {
        rd: u8,
        rn: u8,
        imm: u8,
    },
    SubsImm3 {
        rd: u8,
        rn: u8,
        imm: u8,
    },
    AddsReg {
        rd: u8,
        rn: u8,
        rm: u8,
    },
    SubsReg {
        rd: u8,
        rn: u8,
        rm: u8,
    },
    LslsImm {
        rd: u8,
        rm: u8,
        imm: u8,
    },
    LsrsImm {
        rd: u8,
        rm: u8,
        imm: u8,
    },
    AsrsImm {
        rd: u8,
        rm: u8,
        imm: u8,
    },
    /// MOV/ADD high-register forms, PC excluded at decode time.
    MovHi {
        rd: u8,
        rm: u8,
    },
    AddHi {
        rdn: u8,
        rm: u8,
    },
    Nop,
    // --- Loads/stores ---
    LdrImm {
        rt: u8,
        rn: u8,
        off: u8,
    },
    StrImm {
        rt: u8,
        rn: u8,
        off: u8,
    },
    LdrbImm {
        rt: u8,
        rn: u8,
        off: u8,
    },
    StrbImm {
        rt: u8,
        rn: u8,
        off: u8,
    },
    LdrhImm {
        rt: u8,
        rn: u8,
        off: u8,
    },
    StrhImm {
        rt: u8,
        rn: u8,
        off: u8,
    },
    LdrSp {
        rt: u8,
        off: u16,
    },
    StrSp {
        rt: u8,
        off: u16,
    },
    /// LDR (literal) with the absolute address pre-computed.
    LdrLit {
        rt: u8,
        addr: u32,
    },
    // --- Branches, absolute targets pre-computed ---
    B {
        target: u32,
    },
    BCond {
        cc: u8,
        target: u32,
    },
    Cbz {
        rn: u8,
        target: u32,
    },
    Cbnz {
        rn: u8,
        target: u32,
    },
    /// 32-bit BL (LR is derived from the slot's own address).
    Bl {
        target: u32,
    },
    BxLr,
    // --- Fallbacks: raw halfwords through the existing executors ---
    Narrow(u16),
    Wide(u16, u16),
}

impl DecodedOp {
    /// Decode the instruction at `addr`. `hw2` is only meaningful when
    /// `hw1` selects a 32-bit encoding.
    pub fn decode(hw1: u16, hw2: u16, addr: u32) -> DecodedOp {
        use DecodedOp::*;
        let op = hw1 as u32;
        let pc4 = addr.wrapping_add(4);
        if op >= 0xe800 {
            let h1 = op;
            let h2 = hw2 as u32;
            // BL: the single hottest wide instruction.
            if h1 & 0xf800 == 0xf000 && h2 & 0xd000 == 0xd000 {
                let s = (h1 >> 10) & 1;
                let j1 = (h2 >> 13) & 1;
                let j2 = (h2 >> 11) & 1;
                let i1 = !(j1 ^ s) & 1;
                let i2 = !(j2 ^ s) & 1;
                let imm =
                    (s << 24 | i1 << 23 | i2 << 22 | (h1 & 0x3ff) << 12 | (h2 & 0x7ff) << 1) as i32;
                let offset = imm << 7 >> 7;
                return Bl {
                    target: pc4.wrapping_add(offset as u32),
                };
            }
            return Wide(hw1, hw2);
        }
        match op >> 12 {
            0x0 | 0x1 => {
                let rd = (op & 0x7) as u8;
                let rn = ((op >> 3) & 0x7) as u8;
                let imm5 = ((op >> 6) & 0x1f) as u8;
                match (op >> 11) & 0x3 {
                    0b00 if imm5 != 0 => LslsImm {
                        rd,
                        rm: rn,
                        imm: imm5,
                    },
                    0b01 => LsrsImm {
                        rd,
                        rm: rn,
                        imm: imm5,
                    },
                    0b10 => AsrsImm {
                        rd,
                        rm: rn,
                        imm: imm5,
                    },
                    0b11 => {
                        let rm_or_imm = ((op >> 6) & 0x7) as u8;
                        match (op >> 9) & 0x3 {
                            0b00 => AddsReg {
                                rd,
                                rn,
                                rm: rm_or_imm,
                            },
                            0b01 => SubsReg {
                                rd,
                                rn,
                                rm: rm_or_imm,
                            },
                            0b10 => AddsImm3 {
                                rd,
                                rn,
                                imm: rm_or_imm,
                            },
                            _ => SubsImm3 {
                                rd,
                                rn,
                                imm: rm_or_imm,
                            },
                        }
                    }
                    _ => Narrow(hw1), // LSLS #0 (MOV-with-flags edge case)
                }
            }
            0x2 | 0x3 => {
                let r = ((op >> 8) & 0x7) as u8;
                let imm = (op & 0xff) as u8;
                match (op >> 11) & 0x3 {
                    0b00 => MovsImm8 { rd: r, imm },
                    0b01 => CmpImm8 { rn: r, imm },
                    0b10 => AddsImm8 { rdn: r, imm },
                    _ => SubsImm8 { rdn: r, imm },
                }
            }
            0x4 => {
                if op == 0x4770 {
                    return BxLr;
                }
                if (op >> 10) & 0x3 == 0b01 {
                    // Hi-register MOV/ADD without PC involvement.
                    let rm = ((op >> 3) & 0xf) as u8;
                    let rd = ((op & 0x7) | ((op >> 4) & 0x8)) as u8;
                    if rm != 15 && rd != 15 {
                        match (op >> 8) & 0x3 {
                            0b00 => return AddHi { rdn: rd, rm },
                            0b10 => return MovHi { rd, rm },
                            _ => {}
                        }
                    }
                    return Narrow(hw1);
                }
                if (op >> 11) & 0x1 == 1 {
                    // LDR (literal): absolute address bakes in here.
                    let rt = ((op >> 8) & 0x7) as u8;
                    return LdrLit {
                        rt,
                        addr: (pc4 & !0x3).wrapping_add((op & 0xff) << 2),
                    };
                }
                Narrow(hw1)
            }
            0x6 => {
                let off = (((op >> 6) & 0x1f) << 2) as u8;
                let rn = ((op >> 3) & 0x7) as u8;
                let rt = (op & 0x7) as u8;
                if op & 0x800 != 0 {
                    LdrImm { rt, rn, off }
                } else {
                    StrImm { rt, rn, off }
                }
            }
            0x7 => {
                let off = ((op >> 6) & 0x1f) as u8;
                let rn = ((op >> 3) & 0x7) as u8;
                let rt = (op & 0x7) as u8;
                if op & 0x800 != 0 {
                    LdrbImm { rt, rn, off }
                } else {
                    StrbImm { rt, rn, off }
                }
            }
            0x8 => {
                let off = (((op >> 6) & 0x1f) << 1) as u8;
                let rn = ((op >> 3) & 0x7) as u8;
                let rt = (op & 0x7) as u8;
                if op & 0x800 != 0 {
                    LdrhImm { rt, rn, off }
                } else {
                    StrhImm { rt, rn, off }
                }
            }
            0x9 => {
                let rt = ((op >> 8) & 0x7) as u8;
                let off = ((op & 0xff) << 2) as u16;
                if op & 0x800 != 0 {
                    LdrSp { rt, off }
                } else {
                    StrSp { rt, off }
                }
            }
            0xb => {
                if op == 0xbf00 {
                    return Nop;
                }
                if (op >> 8) & 0xf == 0x1
                    || (op >> 8) & 0xf == 0x3
                    || (op >> 8) & 0xf == 0x9
                    || (op >> 8) & 0xf == 0xb
                {
                    // CBZ/CBNZ (never inside IT blocks by architecture).
                    let rn = (op & 0x7) as u8;
                    let imm = ((op >> 3) & 0x1f) << 1 | ((op >> 9) & 0x1) << 6;
                    let target = pc4.wrapping_add(imm);
                    return if op & 0x800 != 0 {
                        Cbnz { rn, target }
                    } else {
                        Cbz { rn, target }
                    };
                }
                Narrow(hw1)
            }
            0xd => {
                let cc = ((op >> 8) & 0xf) as u8;
                if cc < 0xe {
                    let imm = ((op & 0xff) as i8 as i32) << 1;
                    BCond {
                        cc,
                        target: pc4.wrapping_add(imm as u32),
                    }
                } else {
                    Narrow(hw1) // UDF / SVC
                }
            }
            0xe => {
                // Unconditional B (0xE000..0xE7FF only; >= 0xE800 is wide).
                let imm = ((op & 0x7ff) << 21) as i32 >> 20;
                B {
                    target: pc4.wrapping_add(imm as u32),
                }
            }
            _ => Narrow(hw1),
        }
    }

    /// Instruction size in bytes.
    #[inline(always)]
    pub fn width(self) -> u32 {
        match self {
            DecodedOp::Wide(..) | DecodedOp::Bl { .. } => 4,
            _ => 2,
        }
    }
}

impl CortexM7 {
    /// Execute a decoded op fetched from `pc`. The op sets PC itself.
    pub(super) fn exec_decoded(&mut self, bus: &mut impl Bus, op: DecodedOp, pc: u32) {
        use DecodedOp::*;
        let pc4 = pc.wrapping_add(4);
        self.regs[PC] = pc.wrapping_add(op.width());
        match op {
            MovsImm8 { rd, imm } => {
                let v = imm as u32;
                self.regs[rd as usize] = v;
                self.set_nz(v);
            }
            CmpImm8 { rn, imm } => {
                self.add_update(self.regs[rn as usize], !(imm as u32), true, true);
            }
            AddsImm8 { rdn, imm } => {
                self.regs[rdn as usize] =
                    self.add_update(self.regs[rdn as usize], imm as u32, false, true);
            }
            SubsImm8 { rdn, imm } => {
                self.regs[rdn as usize] =
                    self.add_update(self.regs[rdn as usize], !(imm as u32), true, true);
            }
            AddsImm3 { rd, rn, imm } => {
                self.regs[rd as usize] =
                    self.add_update(self.regs[rn as usize], imm as u32, false, true);
            }
            SubsImm3 { rd, rn, imm } => {
                self.regs[rd as usize] =
                    self.add_update(self.regs[rn as usize], !(imm as u32), true, true);
            }
            AddsReg { rd, rn, rm } => {
                self.regs[rd as usize] =
                    self.add_update(self.regs[rn as usize], self.regs[rm as usize], false, true);
            }
            SubsReg { rd, rn, rm } => {
                self.regs[rd as usize] =
                    self.add_update(self.regs[rn as usize], !self.regs[rm as usize], true, true);
            }
            LslsImm { rd, rm, imm } => {
                self.regs[rd as usize] = self.lsl_c(self.regs[rm as usize], imm as u32, true);
            }
            LsrsImm { rd, rm, imm } => {
                let amount = if imm == 0 { 32 } else { imm as u32 };
                self.regs[rd as usize] = self.lsr_c(self.regs[rm as usize], amount, true);
            }
            AsrsImm { rd, rm, imm } => {
                let amount = if imm == 0 { 32 } else { imm as u32 };
                self.regs[rd as usize] = self.asr_c(self.regs[rm as usize], amount, true);
            }
            MovHi { rd, rm } => self.regs[rd as usize] = self.regs[rm as usize],
            AddHi { rdn, rm } => {
                self.regs[rdn as usize] =
                    self.regs[rdn as usize].wrapping_add(self.regs[rm as usize]);
            }
            Nop => {}
            LdrImm { rt, rn, off } => {
                let addr = self.regs[rn as usize].wrapping_add(off as u32);
                self.regs[rt as usize] = self.read32(bus, addr);
            }
            StrImm { rt, rn, off } => {
                let addr = self.regs[rn as usize].wrapping_add(off as u32);
                let v = self.regs[rt as usize];
                self.write32m(bus, addr, v);
            }
            LdrbImm { rt, rn, off } => {
                let addr = self.regs[rn as usize].wrapping_add(off as u32);
                self.regs[rt as usize] = self.read8m(bus, addr) as u32;
            }
            StrbImm { rt, rn, off } => {
                let addr = self.regs[rn as usize].wrapping_add(off as u32);
                let v = self.regs[rt as usize] as u8;
                self.write8m(bus, addr, v);
            }
            LdrhImm { rt, rn, off } => {
                let addr = self.regs[rn as usize].wrapping_add(off as u32);
                self.regs[rt as usize] = self.read16m(bus, addr) as u32;
            }
            StrhImm { rt, rn, off } => {
                let addr = self.regs[rn as usize].wrapping_add(off as u32);
                let v = self.regs[rt as usize] as u16;
                self.write16m(bus, addr, v);
            }
            LdrSp { rt, off } => {
                let addr = self.regs[SP].wrapping_add(off as u32);
                self.regs[rt as usize] = self.read32(bus, addr);
            }
            StrSp { rt, off } => {
                let addr = self.regs[SP].wrapping_add(off as u32);
                let v = self.regs[rt as usize];
                self.write32m(bus, addr, v);
            }
            LdrLit { rt, addr } => {
                self.regs[rt as usize] = self.read32(bus, addr);
            }
            B { target } => self.regs[PC] = target,
            BCond { cc, target } => {
                if self.cond_passed(cc as u32) {
                    self.regs[PC] = target;
                }
            }
            Cbz { rn, target } => {
                if self.regs[rn as usize] == 0 {
                    self.regs[PC] = target;
                }
            }
            Cbnz { rn, target } => {
                if self.regs[rn as usize] != 0 {
                    self.regs[PC] = target;
                }
            }
            Bl { target } => {
                self.regs[LR] = pc4 | 1;
                self.regs[PC] = target;
            }
            BxLr => {
                let lr = self.regs[LR];
                self.branch_interworking(lr);
            }
            Narrow(hw) => self.exec_narrow(bus, hw, pc4, false),
            Wide(h1, h2) => self.exec_wide(bus, h1, h2, pc4, false),
            NotDecoded => unreachable!("fetch_op never returns NotDecoded"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fast variant must agree with the raw executor on registers
    /// and flags. Run each histogram-hot encoding through both paths.
    #[test]
    fn decoded_matches_raw_executor() {
        use crate::cortex_m::asm;
        let cases: Vec<u16> = vec![
            asm::movs_imm8(3, 0x7f),
            asm::cmp_imm8(3, 0x80),
            asm::adds_imm8(2, 0xff),
            asm::subs_imm8(2, 1),
            asm::adds_imm3(0, 1, 7),
            asm::subs_imm3(0, 1, 7),
            asm::adds_reg(0, 1, 2),
            asm::subs_reg(0, 1, 2),
            asm::lsls_imm(0, 1, 5),
            asm::lsrs_imm(0, 1, 0), // shift-32 edge
            asm::asrs_imm(0, 1, 31),
            asm::mov_hi(8, 1),
            asm::add_hi(8, 1),
            asm::nop(),
            asm::ldr_imm(0, 1, 8),
            asm::str_imm(0, 1, 8),
            asm::ldrb_imm(0, 1, 3),
            asm::strb_imm(0, 1, 3),
            asm::ldrh_imm(0, 1, 6),
            asm::strh_imm(0, 1, 6),
            asm::ldr_sp(0, 16),
            asm::str_sp(0, 16),
            asm::ldr_lit(0, 8),
            asm::b(16),
            asm::b(-16),
            asm::b_cond(0, 8),
            asm::b_cond(11, -8),
            asm::cbz(1, 12),
            asm::cbnz(1, 12),
            asm::bx(14),
        ];
        for &hw in &cases {
            let run = |use_decoded: bool| {
                let mut cpu = CortexM7::new();
                let mut bus = crate::cortex_m::tests::TestBus {
                    ram: vec![0x55; 0x1_0000],
                };
                let base = 0x2000_0100;
                cpu.regs[PC] = base;
                cpu.regs[SP] = 0x2000_8000;
                for r in 0..13 {
                    cpu.regs[r] = 0x1000_0000u32.wrapping_add(0x1111 * r as u32);
                }
                cpu.regs[1] = 0x2000_1000; // valid pointer for loads/stores
                cpu.regs[14] = 0x2000_0203; // thumb return target
                cpu.c = true;
                bus.write16(base, hw);
                if use_decoded {
                    let op = DecodedOp::decode(hw, 0, base);
                    assert_ne!(op, DecodedOp::NotDecoded);
                    cpu.exec_decoded(&mut bus, op, base);
                } else {
                    cpu.step(&mut bus);
                }
                (cpu.regs, cpu.n, cpu.z, cpu.c, cpu.v)
            };
            assert_eq!(run(true), run(false), "mismatch for encoding {hw:#06x}");
        }
    }

    #[test]
    fn bl_decodes_with_absolute_target() {
        let (h1, h2) = crate::cortex_m::asm::bl(-0x20);
        let op = DecodedOp::decode(h1, h2, 0x1000_0100);
        assert_eq!(
            op,
            DecodedOp::Bl {
                target: 0x1000_0104 - 0x20
            }
        );
        assert_eq!(op.width(), 4);
    }
}

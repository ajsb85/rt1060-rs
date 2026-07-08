//! 32-bit Thumb-2 instruction execution (Armv8-M Mainline).
//!
//! Coverage was driven by disassembly of real firmware binaries (see
//! rp2350-rs docs/research/real-binary-analysis.md, where this executor
//! originates); encodings follow the Arm ARM and were cross-checked
//! against GNU `as` output.

use super::{BreakCause, Bus, CortexM7, PC};

/// `Shift_C`: ty 0=LSL 1=LSR 2=ASR 3=ROR 4=RRX. Returns (result, carry_out).
fn shift_c(x: u32, ty: u32, amount: u32, carry_in: bool) -> (u32, bool) {
    match ty {
        0 => {
            if amount == 0 {
                (x, carry_in)
            } else if amount < 32 {
                (x << amount, (x >> (32 - amount)) & 1 != 0)
            } else {
                (0, amount == 32 && x & 1 != 0)
            }
        }
        1 => {
            if amount == 0 {
                (x, carry_in)
            } else if amount < 32 {
                (x >> amount, (x >> (amount - 1)) & 1 != 0)
            } else {
                (0, amount == 32 && x >> 31 != 0)
            }
        }
        2 => {
            if amount == 0 {
                (x, carry_in)
            } else if amount < 32 {
                (((x as i32) >> amount) as u32, (x >> (amount - 1)) & 1 != 0)
            } else {
                (((x as i32) >> 31) as u32, x >> 31 != 0)
            }
        }
        3 => {
            if amount == 0 {
                (x, carry_in)
            } else {
                let m = amount & 31;
                if m == 0 {
                    (x, x >> 31 != 0)
                } else {
                    let r = x.rotate_right(m);
                    (r, r >> 31 != 0)
                }
            }
        }
        _ => ((carry_in as u32) << 31 | x >> 1, x & 1 != 0), // RRX
    }
}

/// `DecodeImmShift`: maps (type, imm5) to a shift_c (ty, amount) pair.
fn decode_imm_shift(ty: u32, imm5: u32) -> (u32, u32) {
    match ty {
        0 => (0, imm5),
        1 => (1, if imm5 == 0 { 32 } else { imm5 }),
        2 => (2, if imm5 == 0 { 32 } else { imm5 }),
        _ => {
            if imm5 == 0 {
                (4, 1) // RRX
            } else {
                (3, imm5)
            }
        }
    }
}

/// `ThumbExpandImm_C`.
fn thumb_expand_imm_c(imm12: u32, carry_in: bool) -> (u32, bool) {
    if imm12 >> 10 == 0 {
        let imm8 = imm12 & 0xff;
        let v = match (imm12 >> 8) & 3 {
            0 => imm8,
            1 => imm8 << 16 | imm8,
            2 => imm8 << 24 | imm8 << 8,
            _ => imm8 << 24 | imm8 << 16 | imm8 << 8 | imm8,
        };
        (v, carry_in)
    } else {
        let unrotated = 0x80 | (imm12 & 0x7f);
        let v = unrotated.rotate_right(imm12 >> 7);
        (v, v >> 31 != 0)
    }
}

/// One lane of a DSP parallel add/subtract. Returns the `width`-bit result
/// (masked, low-aligned) and the lane's GE flag (meaningful only for the plain
/// modifier). `unsigned` selects the sign domain; `sub` the operation; `modif`
/// 0 = plain, 1 = Q (saturate to the type range), 2 = H (arithmetic halve).
fn lane_add_sub(a: u32, b: u32, width: u32, unsigned: bool, sub: bool, modif: u32) -> (u32, bool) {
    let mask = ((1u64 << width) - 1) as u32;
    let (raw, ge): (i64, bool) = if unsigned {
        if sub {
            // GE = 1 when there is no borrow (a >= b).
            (a as i64 - b as i64, a >= b)
        } else {
            let s = a as i64 + b as i64;
            (s, s > mask as i64) // GE = carry out of the lane
        }
    } else {
        let sa = ((a << (32 - width)) as i32 >> (32 - width)) as i64;
        let sb = ((b << (32 - width)) as i32 >> (32 - width)) as i64;
        let r = if sub { sa - sb } else { sa + sb };
        (r, r >= 0) // GE = result is non-negative
    };
    let out = match modif {
        1 => {
            // Q: saturate to the signed or unsigned range of the lane.
            if unsigned {
                raw.clamp(0, mask as i64) as u32
            } else {
                let hi = (mask >> 1) as i64;
                raw.clamp(-hi - 1, hi) as u32
            }
        }
        2 => (raw >> 1) as u32, // H: arithmetic halve
        _ => raw as u32,        // plain
    };
    (out & mask, ge)
}

impl CortexM7 {
    /// Register read where R15 yields the architectural PC (pc4).
    #[inline(always)]
    fn reg_pc(&self, r: usize, pc4: u32) -> u32 {
        if r == PC { pc4 } else { self.regs[r] }
    }

    pub(super) fn exec_wide(
        &mut self,
        bus: &mut impl Bus,
        hw1: u16,
        hw2: u16,
        pc4: u32,
        _in_it: bool,
    ) {
        let h1 = hw1 as u32;
        let h2 = hw2 as u32;
        match h1 >> 11 {
            0b11101 => self.wide_class_a(bus, h1, h2, pc4),
            0b11110 => {
                if h2 & 0x8000 != 0 {
                    self.wide_branch_misc(h1, h2, pc4)
                } else if h1 & 0x0200 == 0 {
                    self.wide_dp_mod_imm(h1, h2)
                } else {
                    self.wide_dp_plain_imm(h1, h2, pc4)
                }
            }
            _ => self.wide_class_c(bus, h1, h2, pc4),
        }
    }

    /// ARMv7E-M DSP parallel add/subtract: operate on the two 16-bit or four
    /// 8-bit lanes of Rn/Rm independently. `op1` (h1[6:4]) picks the lane
    /// layout and per-lane add/sub; `op2` (h2[7:4]) picks signedness (bit 2)
    /// and the modifier (bits 1:0 = plain / Q saturating / H halving). The
    /// plain forms set APSR.GE per lane; Q and H leave GE untouched.
    fn parallel_add_sub(&mut self, op1: u32, op2: u32, rd: usize, rn: usize, rm: usize) {
        let n = self.regs[rn];
        let m = self.regs[rm];
        let unsigned = op2 & 0x4 != 0;
        let modif = op2 & 0x3; // 0 = plain (GE), 1 = Q (saturate), 2 = H (halve)
        let byte = matches!(op1, 0b000 | 0b100); // ADD8 / SUB8
        let mut result = 0u32;
        let mut ge = 0u8;
        if byte {
            let sub = op1 == 0b100;
            for i in 0..4 {
                let a = (n >> (i * 8)) & 0xff;
                let b = (m >> (i * 8)) & 0xff;
                let (lane, g) = lane_add_sub(a, b, 8, unsigned, sub, modif);
                result |= lane << (i * 8);
                if g {
                    ge |= 1 << i;
                }
            }
        } else {
            for i in 0..2 {
                let a = (n >> (i * 16)) & 0xffff;
                // ASX/SAX cross the Rm halfwords; ADD16/SUB16 do not.
                let cross = matches!(op1, 0b010 | 0b110);
                let b = (m >> (if cross { 1 - i } else { i } * 16)) & 0xffff;
                let sub = match op1 {
                    0b001 => false,  // ADD16
                    0b101 => true,   // SUB16
                    0b010 => i == 0, // ASX: low subtracts, high adds
                    _ => i == 1,     // SAX (0b110): low adds, high subtracts
                };
                let (lane, g) = lane_add_sub(a, b, 16, unsigned, sub, modif);
                result |= lane << (i * 16);
                if g {
                    ge |= if i == 0 { 0x3 } else { 0xc };
                }
            }
        }
        self.regs[rd] = result;
        if modif == 0 {
            self.ge = ge;
        }
    }

    /// Shared data-processing core for shifted-register and modified-imm
    /// forms. `op` is instruction bits 24:21; logical ops take C from the
    /// shifter/immediate expansion, arithmetic ops from AddWithCarry.
    fn dp_op(&mut self, op: u32, rn_val: u32, operand: u32, shifter_c: bool, rd: usize, s: bool) {
        let compare_only = rd == PC && s;
        let logical = |cpu: &mut CortexM7, result: u32| {
            if s {
                cpu.n = result >> 31 != 0;
                cpu.z = result == 0;
                cpu.c = shifter_c;
            }
        };
        match op {
            0b0000 => {
                // AND / TST
                let r = rn_val & operand;
                logical(self, r);
                if !compare_only {
                    self.regs[rd] = r;
                }
            }
            0b0001 => {
                let r = rn_val & !operand;
                logical(self, r);
                self.regs[rd] = r;
            }
            0b0010 => {
                // ORR / MOV (Rn == 15)
                let r = rn_val | operand;
                logical(self, r);
                self.regs[rd] = r;
            }
            0b0011 => {
                // ORN / MVN (Rn == 15)
                let r = rn_val | !operand;
                logical(self, r);
                self.regs[rd] = r;
            }
            0b0100 => {
                // EOR / TEQ
                let r = rn_val ^ operand;
                logical(self, r);
                if !compare_only {
                    self.regs[rd] = r;
                }
            }
            0b1000 => {
                // ADD / CMN
                let r = self.add_update(rn_val, operand, false, s);
                if !compare_only {
                    self.regs[rd] = r;
                }
            }
            0b1010 => {
                let c = self.c;
                let r = self.add_update(rn_val, operand, c, s);
                self.regs[rd] = r;
            }
            0b1011 => {
                let c = self.c;
                let r = self.add_update(rn_val, !operand, c, s);
                self.regs[rd] = r;
            }
            0b1101 => {
                // SUB / CMP
                let r = self.add_update(rn_val, !operand, true, s);
                if !compare_only {
                    self.regs[rd] = r;
                }
            }
            0b1110 => {
                let r = self.add_update(!rn_val, operand, true, s);
                self.regs[rd] = r;
            }
            _ => self.break_cause = Some(BreakCause::Unimplemented(op << 21)),
        }
    }

    // --- Class A: 1110 1xxx — LDM/STM, LDRD/STRD, exclusives, TBB, DP reg ---

    fn wide_class_a(&mut self, bus: &mut impl Bus, h1: u32, h2: u32, pc4: u32) {
        if h1 & 0xfe00 == 0xea00 {
            // Data processing, shifted register.
            let op = (h1 >> 5) & 0xf;
            let s = h1 & 0x10 != 0;
            let rn = (h1 & 0xf) as usize;
            let rd = ((h2 >> 8) & 0xf) as usize;
            let rm = (h2 & 0xf) as usize;
            let imm5 = ((h2 >> 12) & 0x7) << 2 | (h2 >> 6) & 0x3;
            let (ty, amount) = decode_imm_shift((h2 >> 4) & 0x3, imm5);
            let (shifted, sc) = shift_c(self.regs[rm], ty, amount, self.c);
            // MOV/MVN forms use Rn=PC as "no first operand".
            let rn_val = if rn == PC {
                match op {
                    0b0010 => 0, // MOV: 0 | operand
                    0b0011 => 0, // MVN: 0 | !operand
                    _ => self.reg_pc(rn, pc4),
                }
            } else {
                self.regs[rn]
            };
            self.dp_op(op, rn_val, shifted, sc, rd, s);
            return;
        }
        if h1 & 0xfff0 == 0xe8d0 {
            let rn = (h1 & 0xf) as usize;
            match h2 & 0xf0 {
                0x00 | 0x10 => {
                    // TBB/TBH: branch table of byte/halfword offsets.
                    let rm = (h2 & 0xf) as usize;
                    let base = self.reg_pc(rn, pc4);
                    let entry = if h2 & 0x10 != 0 {
                        self.read16m(bus, base.wrapping_add(self.regs[rm] << 1)) as u32
                    } else {
                        self.read8m(bus, base.wrapping_add(self.regs[rm])) as u32
                    };
                    self.regs[PC] = pc4.wrapping_add(entry << 1);
                }
                0x40 | 0xc0 => {
                    // LDREXB / LDAEXB (acquire is free: we are seq. consistent).
                    let rt = (h2 >> 12) as usize;
                    self.regs[rt] = self.read8m(bus, self.regs[rn]) as u32;
                }
                0x50 | 0xd0 => {
                    // LDREXH / LDAEXH
                    let rt = (h2 >> 12) as usize;
                    self.regs[rt] = self.read16m(bus, self.regs[rn]) as u32;
                }
                0xe0 => {
                    // LDAEX (word)
                    let rt = (h2 >> 12) as usize;
                    self.regs[rt] = self.read32(bus, self.regs[rn]);
                }
                0x80 => {
                    // LDAB
                    let rt = (h2 >> 12) as usize;
                    self.regs[rt] = self.read8m(bus, self.regs[rn]) as u32;
                }
                0x90 => {
                    // LDAH
                    let rt = (h2 >> 12) as usize;
                    self.regs[rt] = self.read16m(bus, self.regs[rn]) as u32;
                }
                0xa0 => {
                    // LDA
                    let rt = (h2 >> 12) as usize;
                    self.regs[rt] = self.read32(bus, self.regs[rn]);
                }
                _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
            }
            return;
        }
        if h1 & 0xfff0 == 0xe8c0 {
            // Exclusive/release stores: single core, always succeed (Rd = 0).
            let rn = (h1 & 0xf) as usize;
            let rt = (h2 >> 12) as usize;
            let rd = (h2 & 0xf) as usize;
            let addr = self.regs[rn];
            match h2 & 0xf0 {
                0x40 | 0xc0 => self.write8m(bus, addr, self.regs[rt] as u8), // STREXB/STLEXB
                0x50 | 0xd0 => self.write16m(bus, addr, self.regs[rt] as u16), // STREXH/STLEXH
                0xe0 => self.write32m(bus, addr, self.regs[rt]),             // STLEX
                // STLB/STLH/STL: plain release stores, no status register.
                0x80 => {
                    self.write8m(bus, addr, self.regs[rt] as u8);
                    return;
                }
                0x90 => {
                    self.write16m(bus, addr, self.regs[rt] as u16);
                    return;
                }
                0xa0 => {
                    self.write32m(bus, addr, self.regs[rt]);
                    return;
                }
                _ => {
                    self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
                    return;
                }
            }
            self.regs[rd] = 0;
            return;
        }
        if h1 & 0xfe40 == 0xe840 && h1 & 0x120 != 0 {
            // LDRD/STRD (P or W set; P==W==0 is the exclusive space).
            let u = h1 & 0x80 != 0;
            let p = h1 & 0x100 != 0;
            let w = h1 & 0x20 != 0;
            let load = h1 & 0x10 != 0;
            let rn = (h1 & 0xf) as usize;
            let rt = (h2 >> 12) as usize;
            let rt2 = ((h2 >> 8) & 0xf) as usize;
            let imm = (h2 & 0xff) << 2;
            let base = if rn == PC { pc4 & !0x3 } else { self.regs[rn] };
            let offset_addr = if u {
                base.wrapping_add(imm)
            } else {
                base.wrapping_sub(imm)
            };
            let addr = if p { offset_addr } else { base };
            if load {
                let lo = self.read32(bus, addr);
                let hi = self.read32(bus, addr.wrapping_add(4));
                if w && rn != PC {
                    self.regs[rn] = offset_addr;
                }
                self.regs[rt] = lo;
                self.regs[rt2] = hi;
            } else {
                self.write32m(bus, addr, self.regs[rt]);
                self.write32m(bus, addr.wrapping_add(4), self.regs[rt2]);
                if w && rn != PC {
                    self.regs[rn] = offset_addr;
                }
            }
            return;
        }
        if h1 & 0xfff0 == 0xe850 && h2 & 0x0f00 == 0x0f00 {
            // LDREX: single core — always succeeds.
            let rn = (h1 & 0xf) as usize;
            let rt = (h2 >> 12) as usize;
            let addr = self.regs[rn].wrapping_add((h2 & 0xff) << 2);
            self.regs[rt] = self.read32(bus, addr);
            return;
        }
        if h1 & 0xfff0 == 0xe840 {
            if h2 & 0xf03f == 0xf000 {
                // TT/TTT/TTA/TTAT: security attribution query. This emulator
                // models a single Secure world: S=1 (bit 22), no region info
                // (TrustZone-lite, docs/DESIGN.md §2).
                let rd = ((h2 >> 8) & 0xf) as usize;
                self.regs[rd] = 1 << 22;
                return;
            }
            // STREX: always succeeds (Rd = 0).
            let rn = (h1 & 0xf) as usize;
            let rt = (h2 >> 12) as usize;
            let rd = ((h2 >> 8) & 0xf) as usize;
            let addr = self.regs[rn].wrapping_add((h2 & 0xff) << 2);
            self.write32m(bus, addr, self.regs[rt]);
            self.regs[rd] = 0;
            return;
        }
        if h1 & 0xec00 == 0xec00 {
            // Coprocessor space. cp10/11 is the FPU (in-core); everything
            // else routes through the Bus hooks, whose defaults read as 0
            // and ignore writes (the MG24 has no bus-visible coprocessors).
            let cp = (h2 >> 8) & 0xf;
            if cp == 0xa || cp == 0xb {
                self.exec_fp(bus, h1, h2, pc4);
                return;
            }
            if h1 & 0xff00 == 0xee00 && h2 & 0x10 != 0 {
                // MCR/MRC: opc1 = hw1[7:5], CRn = hw1[3:0], opc2 = hw2[7:5],
                // CRm = hw2[3:0], L = hw1[4].
                let opc1 = (h1 >> 5) & 0x7;
                let crn = h1 & 0xf;
                let opc2 = (h2 >> 5) & 0x7;
                let crm = h2 & 0xf;
                let rt = (h2 >> 12) as usize;
                if h1 & 0x10 != 0 {
                    let v = bus.mrc(cp, opc1, crn, crm, opc2);
                    if rt == PC {
                        // APSR_NZCV destination.
                        self.n = v >> 31 != 0;
                        self.z = v >> 30 & 1 != 0;
                        self.c = v >> 29 & 1 != 0;
                        self.v = v >> 28 & 1 != 0;
                    } else {
                        self.regs[rt] = v;
                    }
                } else {
                    bus.mcr(cp, opc1, crn, crm, opc2, self.regs[rt]);
                }
            } else if h1 & 0xffe0 == 0xec40 {
                // MCRR/MRRC: opc1 = hw2[7:4], CRm = hw2[3:0].
                let opc1 = (h2 >> 4) & 0xf;
                let crm = h2 & 0xf;
                let rt = (h2 >> 12) as usize;
                let rt2 = (h1 & 0xf) as usize;
                if h1 & 0x10 != 0 {
                    let (lo, hi) = bus.mrrc(cp, opc1, crm);
                    self.regs[rt] = lo;
                    self.regs[rt2] = hi;
                } else {
                    bus.mcrr(cp, opc1, crm, self.regs[rt], self.regs[rt2]);
                }
            }
            // CDP/LDC/STC and other forms: architectural no-op here.
            return;
        }
        if h1 & 0xfe00 == 0xe800 {
            // LDM/STM wide: mode bits 8:7 — 01 = IA, 10 = DB.
            let mode = (h1 >> 7) & 0x3;
            let w = h1 & 0x20 != 0;
            let load = h1 & 0x10 != 0;
            let rn = (h1 & 0xf) as usize;
            let list = h2 & 0xdfff; // bit 13 reserved
            let count = list.count_ones() * 4;
            let base = self.regs[rn];
            let (mut addr, wb) = match mode {
                0b01 => (base, base.wrapping_add(count)),
                0b10 => (base.wrapping_sub(count), base.wrapping_sub(count)),
                _ => {
                    self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
                    return;
                }
            };
            if load {
                if w && list & (1 << rn) == 0 {
                    self.regs[rn] = wb;
                }
                for i in 0..15 {
                    if list & (1 << i) != 0 {
                        self.regs[i] = self.read32(bus, addr);
                        addr = addr.wrapping_add(4);
                    }
                }
                if list & 0x8000 != 0 {
                    let target = self.read32(bus, addr);
                    self.branch_interworking(target);
                }
            } else {
                for i in 0..16 {
                    if list & (1 << i) != 0 {
                        self.write32m(bus, addr, self.regs[i]);
                        addr = addr.wrapping_add(4);
                    }
                }
                if w {
                    self.regs[rn] = wb;
                }
            }
            return;
        }
        self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
    }

    // --- Class B: data processing with immediates ---------------------------

    fn wide_dp_mod_imm(&mut self, h1: u32, h2: u32) {
        let op = (h1 >> 5) & 0xf;
        let s = h1 & 0x10 != 0;
        let rn = (h1 & 0xf) as usize;
        let rd = ((h2 >> 8) & 0xf) as usize;
        let imm12 = ((h1 >> 10) & 1) << 11 | ((h2 >> 12) & 0x7) << 8 | h2 & 0xff;
        let (imm32, carry) = thumb_expand_imm_c(imm12, self.c);
        let rn_val = if rn == PC { 0 } else { self.regs[rn] };
        self.dp_op(op, rn_val, imm32, carry, rd, s);
    }

    fn wide_dp_plain_imm(&mut self, h1: u32, h2: u32, pc4: u32) {
        let op = (h1 >> 4) & 0x1f;
        let rn = (h1 & 0xf) as usize;
        let rd = ((h2 >> 8) & 0xf) as usize;
        let imm12 = ((h1 >> 10) & 1) << 11 | ((h2 >> 12) & 0x7) << 8 | h2 & 0xff;
        match op {
            0b00000 => {
                // ADDW / ADR.W
                let base = if rn == PC { pc4 & !0x3 } else { self.regs[rn] };
                self.regs[rd] = base.wrapping_add(imm12);
            }
            0b01010 => {
                let base = if rn == PC { pc4 & !0x3 } else { self.regs[rn] };
                self.regs[rd] = base.wrapping_sub(imm12);
            }
            0b00100 => {
                // MOVW: imm16 = imm4:i:imm3:imm8.
                self.regs[rd] = (h1 & 0xf) << 12 | imm12;
            }
            0b01100 => {
                // MOVT: top half, bottom preserved.
                let imm16 = (h1 & 0xf) << 12 | imm12;
                self.regs[rd] = self.regs[rd] & 0xffff | imm16 << 16;
            }
            0b10100 | 0b11100 => {
                // SBFX / UBFX
                let lsb = ((h2 >> 12) & 0x7) << 2 | (h2 >> 6) & 0x3;
                let widthm1 = h2 & 0x1f;
                let unsigned = op == 0b11100;
                let val = self.regs[rn] >> lsb;
                let width = widthm1 + 1;
                let mask = if width == 32 {
                    u32::MAX
                } else {
                    (1 << width) - 1
                };
                self.regs[rd] = if unsigned {
                    val & mask
                } else {
                    let v = val & mask;
                    let sign = 1u32 << widthm1;
                    (v ^ sign).wrapping_sub(sign)
                };
            }
            0b10000 | 0b10010 | 0b11000 | 0b11010 => {
                // SSAT / USAT (Armv8-M F3.4 "plain binary immediate";
                // op bit1 = sh: 0 = LSL #amt, 1 = ASR #amt). Needed by real
                // MG24 firmware: sl_device_init_hfxo_s2.c CTUNE math
                // compiles to `usat r0, #8, r0`.
                let unsigned = op & 0b01000 != 0;
                let sh_asr = op & 0b00010 != 0;
                let amt = ((h2 >> 12) & 0x7) << 2 | (h2 >> 6) & 0x3;
                if sh_asr && amt == 0 {
                    // SSAT16 / USAT16 encoding space (DSP halfword
                    // saturation) — not seen in any fixture yet.
                    self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
                    return;
                }
                let sat_imm = h2 & 0x1f;
                let operand = if sh_asr {
                    (self.regs[rn] as i32) >> amt
                } else {
                    (self.regs[rn] << amt) as i32
                } as i64;
                let (min, max): (i64, i64) = if unsigned {
                    // USAT: n = sat_imm (0..=31), range 0 ..= 2^n - 1.
                    (0, (1i64 << sat_imm) - 1)
                } else {
                    // SSAT: n = sat_imm + 1 (1..=32), range -2^(n-1) ..= 2^(n-1)-1.
                    (-(1i64 << sat_imm), (1i64 << sat_imm) - 1)
                };
                let clamped = operand.clamp(min, max);
                if clamped != operand {
                    self.q = true; // sticky saturation flag
                }
                self.regs[rd] = clamped as u32;
            }
            0b10110 => {
                // BFI / BFC (Rn == 15)
                let lsb = ((h2 >> 12) & 0x7) << 2 | (h2 >> 6) & 0x3;
                let msb = h2 & 0x1f;
                if msb < lsb {
                    return; // UNPREDICTABLE; ignore
                }
                let width = msb - lsb + 1;
                let mask = if width == 32 {
                    u32::MAX
                } else {
                    (1 << width) - 1
                };
                let field = if rn == PC { 0 } else { self.regs[rn] & mask };
                self.regs[rd] = self.regs[rd] & !(mask << lsb) | field << lsb;
            }
            _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
        }
    }

    fn wide_branch_misc(&mut self, h1: u32, h2: u32, pc4: u32) {
        if h2 & 0xd000 == 0xd000 || h2 & 0xd000 == 0x9000 {
            // BL / B.W: imm = S:I1:I2:imm10:imm11:'0' (25-bit).
            let s = (h1 >> 10) & 1;
            let j1 = (h2 >> 13) & 1;
            let j2 = (h2 >> 11) & 1;
            let i1 = !(j1 ^ s) & 1;
            let i2 = !(j2 ^ s) & 1;
            let imm =
                (s << 24 | i1 << 23 | i2 << 22 | (h1 & 0x3ff) << 12 | (h2 & 0x7ff) << 1) as i32;
            let offset = imm << 7 >> 7;
            if h2 & 0x4000 != 0 {
                self.regs[super::LR] = self.regs[PC] | 1;
            }
            self.regs[PC] = pc4.wrapping_add(offset as u32);
            return;
        }
        if h2 & 0xd000 == 0x8000 {
            let cond = (h1 >> 6) & 0xf;
            if cond < 0b1110 {
                // Conditional B.W: imm = S:J2:J1:imm6:imm11:'0' (21-bit).
                if self.cond_passed(cond) {
                    let s = (h1 >> 10) & 1;
                    let j1 = (h2 >> 13) & 1;
                    let j2 = (h2 >> 11) & 1;
                    let imm =
                        (s << 20 | j2 << 19 | j1 << 18 | (h1 & 0x3f) << 12 | (h2 & 0x7ff) << 1)
                            as i32;
                    let offset = imm << 11 >> 11;
                    self.regs[PC] = pc4.wrapping_add(offset as u32);
                }
                return;
            }
            // cond = 111x: system instructions.
            if h1 == 0xf3bf && h2 & 0xff00 == 0x8f00 {
                return; // DMB/DSB/ISB: memory is sequentially consistent
            }
            if h1 == 0xf3af && h2 & 0xf000 == 0x8000 {
                // Wide hints: NOP.W/YIELD/WFE/WFI/SEV/DBG (T32 F3AF 80xx).
                // GCC emits nop.w for alignment (seen in
                // sl_power_manager_sleep); all are no-ops here.
                return;
            }
            if h1 & 0xffe0 == 0xf3e0 {
                // MRS Rd, spec_reg
                let rd = ((h2 >> 8) & 0xf) as usize;
                self.regs[rd] = match h2 & 0xff {
                    0..=3 => self.apsr() | if h2 & 0x1 != 0 { self.ipsr as u32 } else { 0 },
                    8 => self.msp(),
                    9 => self.psp(),
                    16 => self.primask as u32,
                    17 | 18 => self.basepri as u32,
                    19 => self.faultmask as u32,
                    20 => {
                        (self.npriv as u32) | ((self.sp_sel == super::SpSel::Process) as u32) << 1
                    }
                    _ => 0,
                };
                return;
            }
            if h1 & 0xffe0 == 0xf380 {
                // MSR spec_reg, Rn
                let value = self.regs[(h1 & 0xf) as usize];
                match h2 & 0xff {
                    0..=3 => self.set_apsr(value),
                    8 => self.set_msp(value),
                    9 => self.set_psp(value),
                    16 => self.primask = value & 1 != 0,
                    17 | 18 => self.basepri = value as u8,
                    19 => self.faultmask = value & 1 != 0,
                    20 => {
                        self.npriv = value & 1 != 0;
                        let want = if value & 2 != 0 {
                            super::SpSel::Process
                        } else {
                            super::SpSel::Main
                        };
                        self.switch_sp(want);
                    }
                    _ => {}
                }
                return;
            }
        }
        self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
    }

    // --- Class C: load/store single, shifts, multiply -----------------------

    fn wide_class_c(&mut self, bus: &mut impl Bus, h1: u32, h2: u32, pc4: u32) {
        if h1 & 0xfe00 == 0xf800 {
            self.wide_load_store(bus, h1, h2, pc4);
            return;
        }
        if h1 & 0xff00 == 0xfe00 && (h2 >> 8) & 0xe == 0xa {
            // FE-space FP: VSEL/VMAXNM/VMINNM/VRINT/VCVT{A,N,P,M}.
            self.exec_fp(bus, h1, h2, pc4);
            return;
        }
        if h1 & 0xff80 == 0xfa00 {
            let rn = (h1 & 0xf) as usize;
            let rd = ((h2 >> 8) & 0xf) as usize;
            let rm = (h2 & 0xf) as usize;
            if h2 & 0x80 != 0 {
                // Extend family with rotation: SXTH/UXTH/SXTB/UXTB when
                // Rn == PC, else the accumulate forms SXTAH/UXTAH/
                // SXTAB/UXTAB (Rd = Rn + extended). Getting this wrong is
                // catastrophic: a missed decode here once fell through to
                // the register-shift branch and zeroed tinyusb's return
                // values via `uxtah`.
                let rotation = ((h2 >> 4) & 0x3) * 8;
                let x = self.regs[rm].rotate_right(rotation);
                let extended = match (h1 >> 4) & 0x7 {
                    0b000 => x as i16 as i32 as u32,
                    0b001 => x & 0xffff,
                    0b100 => x as i8 as i32 as u32,
                    0b101 => x & 0xff,
                    _ => {
                        self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
                        return;
                    }
                };
                let acc = if rn == PC { 0 } else { self.regs[rn] };
                self.regs[rd] = extended.wrapping_add(acc);
                return;
            }
            // LSL/LSR/ASR/ROR (register) wide.
            let ty = (h1 >> 5) & 0x3;
            let s = h1 & 0x10 != 0;
            let (r, c) = shift_c(self.regs[rn], ty, self.regs[rm] & 0xff, self.c);
            self.regs[rd] = r;
            if s {
                self.n = r >> 31 != 0;
                self.z = r == 0;
                self.c = c;
            }
            return;
        }
        if h1 & 0xff80 == 0xfa80 && h2 & 0xf000 == 0xf000 {
            let rn = (h1 & 0xf) as usize;
            let rd = ((h2 >> 8) & 0xf) as usize;
            let rm = (h2 & 0xf) as usize;
            let op1 = (h1 >> 4) & 0x7; // h1[6:4]
            let op2 = (h2 >> 4) & 0xf; // h2[7:4]
            if op2 & 0x8 == 0 {
                // Parallel add/subtract (signed/unsigned, byte/halfword),
                // ARMv7E-M DSP. op1 selects add8/add16/asx/sub8/sub16/sax;
                // op2[2] = unsigned, op2[1:0] = plain(GE) / Q(sat) / H(halve).
                self.parallel_add_sub(op1, op2, rd, rn, rm);
                return;
            }
            // Miscellaneous: SEL, then CLZ / RBIT / REV.W family.
            let x = self.regs[rn];
            let result = match (op1, op2) {
                (0b010, 0b1000) => {
                    // SEL: pick each byte from Rn if the matching GE flag is
                    // set, else from Rm (Rn == self.regs[rn] == x).
                    let y = self.regs[rm];
                    let mut r = 0u32;
                    for i in 0..4 {
                        let lane = 0xffu32 << (i * 8);
                        r |= if self.ge & (1 << i) != 0 { x } else { y } & lane;
                    }
                    r
                }
                (0b011, 0b1000) => x.leading_zeros(), // CLZ
                (0b001, 0b1010) => x.reverse_bits(),  // RBIT
                (0b001, 0b1000) => x.swap_bytes(),    // REV.W
                (0b001, 0b1001) => (x & 0xff00_ff00) >> 8 | (x & 0x00ff_00ff) << 8, // REV16.W
                (0b001, 0b1011) => (x as u16).swap_bytes() as i16 as i32 as u32, // REVSH.W
                _ => {
                    self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
                    return;
                }
            };
            self.regs[rd] = result;
            return;
        }
        if h1 & 0xff00 == 0xfb00 {
            // Multiply / divide.
            let rn = (h1 & 0xf) as usize;
            let ra = ((h2 >> 12) & 0xf) as usize;
            let rd = ((h2 >> 8) & 0xf) as usize;
            let rm = (h2 & 0xf) as usize;
            let op = (h1 >> 4) & 0xf;
            let op2 = (h2 >> 4) & 0xf;
            let x = self.regs[rn];
            let y = self.regs[rm];
            match (op, op2) {
                (0b0000, 0b0000) => {
                    // MLA (MUL when Ra == 15).
                    let acc = if ra == PC { 0 } else { self.regs[ra] };
                    self.regs[rd] = x.wrapping_mul(y).wrapping_add(acc);
                }
                (0b0000, 0b0001) => {
                    self.regs[rd] = self.regs[ra].wrapping_sub(x.wrapping_mul(y));
                }
                (0b0001, 0b0000..=0b0011) => {
                    // SMLABB/BT/TB/TT (SMULxy when Ra == 15): signed
                    // halfword multiply, optional accumulate. Q flag
                    // saturation state is not modeled.
                    let xh = if op2 & 0b10 != 0 { x >> 16 } else { x } as u16 as i16 as i32;
                    let yh = if op2 & 0b01 != 0 { y >> 16 } else { y } as u16 as i16 as i32;
                    let acc = if ra == PC { 0 } else { self.regs[ra] };
                    self.regs[rd] = (xh.wrapping_mul(yh) as u32).wrapping_add(acc);
                }
                (0b0011, 0b0000 | 0b0001) => {
                    // SMULWB/WT (SMLAW with Ra != 15): (Rn * half) >> 16.
                    let yh = if op2 & 0b01 != 0 { y >> 16 } else { y } as u16 as i16 as i64;
                    let acc = if ra == PC { 0 } else { self.regs[ra] };
                    let prod = ((x as i32 as i64).wrapping_mul(yh) >> 16) as u32;
                    self.regs[rd] = prod.wrapping_add(acc);
                }
                (0b0101, 0b0000 | 0b0001) => {
                    // SMMUL/SMMLA (+R rounding): high word of Rn * Rm.
                    let acc = if ra == PC {
                        0i64
                    } else {
                        (self.regs[ra] as i64) << 32
                    };
                    let mut v = acc.wrapping_add((x as i32 as i64).wrapping_mul(y as i32 as i64));
                    if op2 & 1 != 0 {
                        v = v.wrapping_add(0x8000_0000);
                    }
                    self.regs[rd] = (v >> 32) as u32;
                }
                (0b0110, 0b0000 | 0b0001) => {
                    // SMMLS (+R): Ra:0 - Rn*Rm, high word.
                    let mut v = ((self.regs[ra] as i64) << 32)
                        .wrapping_sub((x as i32 as i64).wrapping_mul(y as i32 as i64));
                    if op2 & 1 != 0 {
                        v = v.wrapping_add(0x8000_0000);
                    }
                    self.regs[rd] = (v >> 32) as u32;
                }
                (0b1000, 0b0000) => {
                    let r = (x as i32 as i64).wrapping_mul(y as i32 as i64) as u64;
                    self.regs[ra] = r as u32;
                    self.regs[rd] = (r >> 32) as u32;
                }
                (0b1010, 0b0000) => {
                    let r = (x as u64).wrapping_mul(y as u64);
                    self.regs[ra] = r as u32;
                    self.regs[rd] = (r >> 32) as u32;
                }
                (0b1100, 0b0000) => {
                    // SMLAL
                    let acc = ((self.regs[rd] as u64) << 32 | self.regs[ra] as u64) as i64;
                    let r =
                        acc.wrapping_add((x as i32 as i64).wrapping_mul(y as i32 as i64)) as u64;
                    self.regs[ra] = r as u32;
                    self.regs[rd] = (r >> 32) as u32;
                }
                (0b1110, 0b0000) => {
                    // UMLAL
                    let acc = (self.regs[rd] as u64) << 32 | self.regs[ra] as u64;
                    let r = acc.wrapping_add((x as u64).wrapping_mul(y as u64));
                    self.regs[ra] = r as u32;
                    self.regs[rd] = (r >> 32) as u32;
                }
                (0b1001, 0b1111) => {
                    // SDIV: divide-by-zero yields 0, overflow wraps.
                    self.regs[rd] = if y == 0 {
                        0
                    } else {
                        (x as i32).wrapping_div(y as i32) as u32
                    };
                }
                (0b1011, 0b1111) => {
                    self.regs[rd] = if y == 0 { 0 } else { x / y };
                }
                _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
            }
            return;
        }
        self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
    }

    fn wide_load_store(&mut self, bus: &mut impl Bus, h1: u32, h2: u32, pc4: u32) {
        let signed = h1 & 0x100 != 0;
        let size = (h1 >> 5) & 0x3;
        let load = h1 & 0x10 != 0;
        let rn = (h1 & 0xf) as usize;
        let rt = (h2 >> 12) as usize;
        if size == 3 || (signed && !load) {
            self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
            return;
        }

        let mut writeback: Option<u32> = None;
        let addr = if rn == PC {
            // Literal: U is hw1 bit 7.
            let base = pc4 & !0x3;
            let imm = h2 & 0xfff;
            if h1 & 0x80 != 0 {
                base.wrapping_add(imm)
            } else {
                base.wrapping_sub(imm)
            }
        } else if h1 & 0x80 != 0 {
            self.regs[rn].wrapping_add(h2 & 0xfff)
        } else if h2 & 0x800 != 0 {
            // imm8 with P/U/W (post-index when P=0).
            let p = h2 & 0x400 != 0;
            let u = h2 & 0x200 != 0;
            let w = h2 & 0x100 != 0;
            let imm = h2 & 0xff;
            let base = self.regs[rn];
            let offset_addr = if u {
                base.wrapping_add(imm)
            } else {
                base.wrapping_sub(imm)
            };
            if w || !p {
                writeback = Some(offset_addr);
            }
            if p { offset_addr } else { base }
        } else if h2 & 0xfc0 == 0 {
            // Register with LSL #imm2.
            let rm = (h2 & 0xf) as usize;
            let shift = (h2 >> 4) & 0x3;
            self.regs[rn].wrapping_add(self.regs[rm] << shift)
        } else {
            self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
            return;
        };

        if load {
            if rt == PC {
                if size == 2 {
                    if let Some(wb) = writeback {
                        self.regs[rn] = wb;
                    }
                    let target = self.read32(bus, addr);
                    self.branch_interworking(target);
                }
                // Byte/half with Rt=15 are PLD/PLI hints: nop.
                return;
            }
            let value = match (size, signed) {
                (0, false) => self.read8m(bus, addr) as u32,
                (0, true) => self.read8m(bus, addr) as i8 as i32 as u32,
                (1, false) => self.read16m(bus, addr) as u32,
                (1, true) => self.read16m(bus, addr) as i16 as i32 as u32,
                _ => self.read32(bus, addr),
            };
            if let Some(wb) = writeback {
                self.regs[rn] = wb;
            }
            self.regs[rt] = value; // load value wins over writeback if rt == rn
        } else {
            let value = self.regs[rt];
            match size {
                0 => self.write8m(bus, addr, value as u8),
                1 => self.write16m(bus, addr, value as u16),
                _ => self.write32m(bus, addr, value),
            }
            if let Some(wb) = writeback {
                self.regs[rn] = wb;
            }
        }
    }
}

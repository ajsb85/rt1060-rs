//! FPv5-D16 floating point — the RT1062 has the full double-precision unit.
//!
//! Single-precision is the common path; the double-precision D16 arithmetic
//! (VADD/VSUB/VMUL/VDIV/VABS/VNEG/VSQRT/VCMP.F64 and the cross-precision /
//! integer VCVTs) is also modelled because even soft-float SDK firmware hits
//! it — e.g. `CLOCK_GetPllFreq` computes its PLL ratio as an f64 `(a*b)/c`.
//! The 16 double registers `D0..D15` alias the `S0..S31` pairs
//! (`Dn = S2n:S2n+1`), so they share the `fpregs` file. FP context is never
//! stacked on exception entry (handlers using FP would corrupt thread FP
//! state; SDK handlers don't use FP).
//!
//! Encodings cross-checked against GNU `as -mcpu=cortex-m7 -mfpu=fpv5-d16`.

use super::{BreakCause, Bus, CortexM7, PC};

impl CortexM7 {
    #[inline(always)]
    fn s(&self, r: u32) -> f32 {
        f32::from_bits(self.fpregs[(r & 31) as usize])
    }

    #[inline(always)]
    fn set_s(&mut self, r: u32, v: f32) {
        self.fpregs[(r & 31) as usize] = v.to_bits();
    }

    /// A double register `Dn` aliases the S-register pair `S2n:S2n+1`
    /// (little-endian: the low word is the even S register).
    #[inline(always)]
    fn d(&self, r: u32) -> f64 {
        let lo = self.fpregs[((r * 2) & 31) as usize] as u64;
        let hi = self.fpregs[((r * 2 + 1) & 31) as usize] as u64;
        f64::from_bits(lo | hi << 32)
    }

    #[inline(always)]
    fn set_d(&mut self, r: u32, v: f64) {
        let bits = v.to_bits();
        self.fpregs[((r * 2) & 31) as usize] = bits as u32;
        self.fpregs[((r * 2 + 1) & 31) as usize] = (bits >> 32) as u32;
    }

    /// Double-register number for the Dd/Dn/Dm fields: the extra bit is the
    /// high bit (`hi:v4`), unlike single regs where it is the low bit.
    #[inline(always)]
    fn dreg(hi: u32, v4: u32) -> u32 {
        (hi & 1) << 4 | (v4 & 0xf)
    }

    /// Sd for single ops: Vd:D (register number in bits, D is the low bit).
    fn sd(h1: u32, h2: u32) -> u32 {
        ((h2 >> 12) & 0xf) << 1 | (h1 >> 6) & 1
    }

    fn sn(h1: u32, h2: u32) -> u32 {
        (h1 & 0xf) << 1 | (h2 >> 7) & 1
    }

    fn sm(h2: u32) -> u32 {
        (h2 & 0xf) << 1 | (h2 >> 5) & 1
    }

    /// Execute a cp10/cp11 instruction. Returns without setting a break
    /// cause on success; unknown encodings break loudly.
    pub(super) fn exec_fp(&mut self, bus: &mut impl Bus, h1: u32, h2: u32, pc4: u32) {
        let dp = h2 & 0x100 != 0; // cp11: 64-bit elements

        // --- FE space: unconditional FP (VSEL/VMAXNM/VMINNM/VRINT/VCVT) --
        if h1 & 0xff00 == 0xfe00 {
            if dp {
                self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16));
                return;
            }
            let d = Self::sd(h1, h2);
            let n = Self::sn(h1, h2);
            let m = Self::sm(h2);
            match (h1 >> 4) & 0xf {
                cc @ 0x0..=0x7 => {
                    // VSEL<cc>: condition in h1 bits 5:4 (bit 6 is the D
                    // register bit): 00 EQ, 01 VS, 10 GE, 11 GT.
                    // Selection uses the APSR flags.
                    let take = match cc & 0x3 {
                        0 => self.z,
                        1 => self.v,
                        2 => self.n == self.v,
                        _ => self.n == self.v && !self.z,
                    };
                    let v = if take { self.s(n) } else { self.s(m) };
                    self.set_s(d, v);
                }
                0x8 | 0xc => {
                    // VMAXNM / VMINNM (op = h2 bit 6); D bit may set 0x4.
                    let (a, b) = (self.s(n), self.s(m));
                    let v = if h2 & 0x40 != 0 { a.min(b) } else { a.max(b) };
                    self.set_s(d, v);
                }
                0xb | 0xf => {
                    let v = self.s(m);
                    match h1 & 0xf {
                        0x8 => self.set_s(d, v.round()),           // VRINTA (ties away)
                        0x9 => self.set_s(d, v.round_ties_even()), // VRINTN
                        0xa => self.set_s(d, v.ceil()),            // VRINTP
                        0xb => self.set_s(d, v.floor()),           // VRINTM
                        0xc..=0xf => {
                            // VCVT{A,N,P,M}.{S32,U32}.F32 (h2 bit7: signed).
                            let r = match h1 & 0x3 {
                                0 => v.round(),
                                1 => v.round_ties_even(),
                                2 => v.ceil(),
                                _ => v.floor(),
                            };
                            let bits = if h2 & 0x80 != 0 {
                                r as i32 as u32
                            } else {
                                r as u32
                            };
                            self.fpregs[(d & 31) as usize] = bits;
                        }
                        _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
                    }
                }
                _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
            }
            return;
        }

        // --- Loads/stores (LDC/STC space: h1 = 1110 110P UDWL nnnn) -----
        if h1 & 0xfe00 == 0xec00 {
            let p = h1 & 0x100 != 0;
            let u = h1 & 0x80 != 0;
            let w = h1 & 0x20 != 0;
            let load = h1 & 0x10 != 0;
            let rn = (h1 & 0xf) as usize;

            if !p && !u && !w && h1 & 0x40 != 0 {
                // VMOV two GP regs <-> Sm,Sm+1 (or Dm): EC4x/EC5x
                // (op pattern P=0 U=0 D=1 W=0).
                let rt = ((h2 >> 12) & 0xf) as usize;
                let rt2 = (h1 & 0xf) as usize;
                let m = if dp {
                    (((h2 >> 5) & 1) << 4 | (h2 & 0xf)) << 1
                } else {
                    Self::sm(h2)
                };
                if load {
                    self.regs[rt] = self.fpregs[(m & 31) as usize];
                    self.regs[rt2] = self.fpregs[((m + 1) & 31) as usize];
                } else {
                    self.fpregs[(m & 31) as usize] = self.regs[rt];
                    self.fpregs[((m + 1) & 31) as usize] = self.regs[rt2];
                }
                return;
            }

            // VLDR/VSTR (P=1, W=0) and VLDM/VSTM/VPUSH/VPOP.
            let imm8 = h2 & 0xff;
            let first = if dp {
                (((h1 >> 6) & 1) << 4 | (h2 >> 12) & 0xf) << 1
            } else {
                Self::sd(h1, h2)
            };
            let words = if dp { imm8 & !1 } else { imm8 };
            let base = if rn == PC { pc4 & !0x3 } else { self.regs[rn] };
            if p && !w {
                // VLDR/VSTR: single element (1 word for S, 2 for D).
                let n = if dp { 2 } else { 1 };
                let addr = if u {
                    base.wrapping_add(imm8 << 2)
                } else {
                    base.wrapping_sub(imm8 << 2)
                };
                for i in 0..n {
                    let r = ((first + i) & 31) as usize;
                    if load {
                        self.fpregs[r] = self.read32(bus, addr + 4 * i);
                    } else {
                        let v = self.fpregs[r];
                        self.write32m(bus, addr + 4 * i, v);
                    }
                }
                return;
            }
            // VLDM/VSTM (incl. VPUSH = VSTMDB sp!, VPOP = VLDMIA sp!).
            let count = words.max(1);
            let start = if u {
                base
            } else {
                base.wrapping_sub(count << 2)
            };
            for i in 0..count {
                let r = ((first + i) & 31) as usize;
                let addr = start.wrapping_add(i << 2);
                if load {
                    self.fpregs[r] = self.read32(bus, addr);
                } else {
                    let v = self.fpregs[r];
                    self.write32m(bus, addr, v);
                }
            }
            if w {
                self.regs[rn] = if u {
                    base.wrapping_add(count << 2)
                } else {
                    start
                };
            }
            return;
        }

        // --- Register transfers (bit4 = 1): VMOV/VMRS/VMSR --------------
        if h1 & 0xff00 == 0xee00 && h2 & 0x10 != 0 {
            let rt = ((h2 >> 12) & 0xf) as usize;
            match (h1 >> 4) & 0xf {
                0x0 => {
                    // VMOV Sn, Rt
                    let n = Self::sn(h1, h2);
                    self.fpregs[(n & 31) as usize] = self.regs[rt];
                }
                0x1 => {
                    // VMOV Rt, Sn
                    let n = Self::sn(h1, h2);
                    self.regs[rt] = self.fpregs[(n & 31) as usize];
                }
                0xe => {
                    // VMSR FPSCR, Rt
                    self.fpscr = self.regs[rt];
                }
                0xf => {
                    // VMRS Rt, FPSCR (Rt = 15: APSR_nzcv <- FPSCR flags).
                    if rt == PC {
                        self.n = self.fpscr & 1 << 31 != 0;
                        self.z = self.fpscr & 1 << 30 != 0;
                        self.c = self.fpscr & 1 << 29 != 0;
                        self.v = self.fpscr & 1 << 28 != 0;
                    } else {
                        self.regs[rt] = self.fpscr;
                    }
                }
                _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
            }
            return;
        }

        // --- Data processing (CDP space, bit4 = 0) ----------------------
        if dp {
            // FPv5-D16 double-precision. The RT1062 FPU has the D16 double
            // path (soft-float SDK firmware still hits it in e.g.
            // CLOCK_GetPllFreq's f64 (a*b)/c). D registers alias the S pairs.
            let dd = Self::dreg((h1 >> 6) & 1, (h2 >> 12) & 0xf);
            let dn = Self::dreg((h2 >> 7) & 1, h1 & 0xf);
            let dm = Self::dreg((h2 >> 5) & 1, h2 & 0xf);
            let neg = h2 & 0x40 != 0;
            match (h1 >> 4) & 0xb {
                0x0 => {
                    // VMLA/VMLS.F64
                    let prod = self.d(dn) * self.d(dm);
                    let v = if neg {
                        self.d(dd) - prod
                    } else {
                        self.d(dd) + prod
                    };
                    self.set_d(dd, v);
                }
                0x2 => {
                    let v = self.d(dn) * self.d(dm);
                    self.set_d(dd, if neg { -v } else { v }); // VMUL / VNMUL
                }
                0x3 => {
                    let v = if neg {
                        self.d(dn) - self.d(dm) // VSUB
                    } else {
                        self.d(dn) + self.d(dm) // VADD
                    };
                    self.set_d(dd, v);
                }
                0x8 => self.set_d(dd, self.d(dn) / self.d(dm)), // VDIV
                0xb => {
                    // Extension group: VMOV/VABS/VNEG/VSQRT/VCMP and the
                    // cross-precision / integer VCVTs (the int/single operand
                    // lives in a *single* register, hence sd/sm here).
                    match h1 & 0xf {
                        0x0 => {
                            let v = self.d(dm);
                            self.set_d(dd, if h2 & 0x80 != 0 { v.abs() } else { v }); // VMOV/VABS
                        }
                        0x1 => {
                            let v = self.d(dm);
                            self.set_d(dd, if h2 & 0x80 != 0 { v.sqrt() } else { -v }); // VSQRT/VNEG
                        }
                        0x4 | 0x5 => {
                            // VCMP{,E}.F64 Dd, Dm / #0.0
                            let a = self.d(dd);
                            let b = if h1 & 1 != 0 { 0.0 } else { self.d(dm) };
                            let (nf, zf, cf, vf) = if a.is_nan() || b.is_nan() {
                                (false, false, true, true)
                            } else if a == b {
                                (false, true, true, false)
                            } else if a < b {
                                (true, false, false, false)
                            } else {
                                (false, false, true, false)
                            };
                            self.fpscr = self.fpscr & 0x0fff_ffff
                                | (nf as u32) << 31
                                | (zf as u32) << 30
                                | (cf as u32) << 29
                                | (vf as u32) << 28;
                        }
                        0x7 => {
                            // VCVT.F32.F64: double -> single (Sd single dest).
                            let sd = Self::sd(h1, h2);
                            self.set_s(sd, self.d(dm) as f32);
                        }
                        0x8 => {
                            // VCVT.F64.{U32,S32}: single-int Sm -> double Dd.
                            let sm = Self::sm(h2);
                            let bits = self.fpregs[(sm & 31) as usize];
                            let v = if h2 & 0x80 != 0 {
                                bits as i32 as f64
                            } else {
                                bits as f64
                            };
                            self.set_d(dd, v);
                        }
                        0xc | 0xd => {
                            // VCVT.{U32,S32}.F64: double Dm -> single-int Sd,
                            // round toward zero (Rust `as` saturates like FPU).
                            let sd = Self::sd(h1, h2);
                            let v = self.d(dm);
                            let bits = if h1 & 1 != 0 {
                                v as i32 as u32
                            } else {
                                v as u32
                            };
                            self.fpregs[(sd & 31) as usize] = bits;
                        }
                        _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
                    }
                }
                _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
            }
            return;
        }
        let d = Self::sd(h1, h2);
        let n = Self::sn(h1, h2);
        let m = Self::sm(h2);
        let opc1 = (h1 >> 4) & 0xb; // fold D bit out of bits 7:4
        let neg_m = h2 & 0x40 != 0; // opc3 bit 1 ("op" bit in many forms)
        match opc1 {
            0x0 => {
                // VMLA/VMLS: Sd +=/-= Sn * Sm (separate rounding — close
                // enough to fused for emulation purposes).
                let prod = self.s(n) * self.s(m);
                let v = if neg_m {
                    self.s(d) - prod
                } else {
                    self.s(d) + prod
                };
                self.set_s(d, v);
            }
            0x1 => {
                // VNMLS/VNMLA
                let prod = self.s(n) * self.s(m);
                let v = if neg_m {
                    -self.s(d) - prod
                } else {
                    -self.s(d) + prod
                };
                self.set_s(d, v);
            }
            0x2 => {
                let v = self.s(n) * self.s(m);
                self.set_s(d, if neg_m { -v } else { v }); // VMUL / VNMUL
            }
            0x3 => {
                let v = if neg_m {
                    self.s(n) - self.s(m) // VSUB
                } else {
                    self.s(n) + self.s(m) // VADD
                };
                self.set_s(d, v);
            }
            0x8 => self.set_s(d, self.s(n) / self.s(m)), // VDIV
            0x9 => {
                // VFNMA/VFNMS
                let v = if neg_m {
                    (-self.s(n)).mul_add(self.s(m), -self.s(d))
                } else {
                    self.s(n).mul_add(self.s(m), -self.s(d))
                };
                self.set_s(d, v);
            }
            0xa => {
                // VFMA/VFMS (fused)
                let sn = if neg_m { -self.s(n) } else { self.s(n) };
                let v = sn.mul_add(self.s(m), self.s(d));
                self.set_s(d, v);
            }
            0xb => {
                // Extension group, selected by opc2 (h1 bits 3:0) + opc3.
                if h2 & 0x40 == 0 {
                    // VMOV.F32 #imm — VFPExpandImm(imm4H:imm4L):
                    // sign = b7; exp = NOT(b6):b6 x5:b5:b4; frac = b3..0.
                    let imm8 = (h1 & 0xf) << 4 | h2 & 0xf;
                    let sign = imm8 >> 7;
                    let exp8 = if imm8 & 0x40 != 0 {
                        0x7c | (imm8 >> 4) & 3
                    } else {
                        0x80 | (imm8 >> 4) & 3
                    };
                    self.fpregs[(d & 31) as usize] = sign << 31 | exp8 << 23 | (imm8 & 0xf) << 19;
                    return;
                }
                match h1 & 0xf {
                    0x0 => {
                        // VMOV (reg) / VABS
                        let v = if h2 & 0x80 != 0 {
                            self.s(m).abs()
                        } else {
                            self.s(m)
                        };
                        self.set_s(d, v);
                    }
                    0x1 => {
                        // VNEG / VSQRT
                        let v = if h2 & 0x80 != 0 {
                            self.s(m).sqrt()
                        } else {
                            -self.s(m)
                        };
                        self.set_s(d, v);
                    }
                    0x6 | 0x7 => {
                        // VRINTZ/VRINTR (h2 bit7 set: round toward zero).
                        let v = self.s(m);
                        let r = if h2 & 0x80 != 0 {
                            v.trunc()
                        } else {
                            v.round_ties_even()
                        };
                        self.set_s(d, r);
                    }
                    0x4 | 0x5 => {
                        // VCMP{,E} Sd, Sm / #0.0
                        let a = self.s(d);
                        let b = if h1 & 1 != 0 { 0.0 } else { self.s(m) };
                        let (nf, zf, cf, vf) = if a.is_nan() || b.is_nan() {
                            (false, false, true, true) // unordered
                        } else if a == b {
                            (false, true, true, false)
                        } else if a < b {
                            (true, false, false, false)
                        } else {
                            (false, false, true, false)
                        };
                        self.fpscr = self.fpscr & 0x0fff_ffff
                            | (nf as u32) << 31
                            | (zf as u32) << 30
                            | (cf as u32) << 29
                            | (vf as u32) << 28;
                    }
                    0x8 => {
                        // VCVT.F32.{U32,S32}: int -> float (op bit7: signed).
                        let bits = self.fpregs[(m & 31) as usize];
                        let v = if h2 & 0x80 != 0 {
                            bits as i32 as f32
                        } else {
                            bits as f32
                        };
                        self.set_s(d, v);
                    }
                    0xc | 0xd => {
                        // VCVT.{U32,S32}.F32, round toward zero (Z=bit7 set
                        // in the common form). Rust `as` saturates like the
                        // FPU does.
                        let v = self.s(m);
                        let bits = if h1 & 1 != 0 {
                            v as i32 as u32 // VCVT.S32
                        } else {
                            v as u32 // VCVT.U32
                        };
                        self.fpregs[(d & 31) as usize] = bits;
                    }
                    0xa | 0xb | 0xe | 0xf => {
                        // VCVT fixed-point ↔ F32, in place on Sd (Arm ARM
                        // A7.7.226 VCVT between FP and fixed-point): opc2
                        // 0xA/0xB = fixed→float (signed/unsigned), 0xE/0xF =
                        // float→fixed. sx (h2 bit7): 0 = 16-bit, 1 = 32-bit
                        // container; fbits = size − imm4:i. Real firmware:
                        // IADC_init compiles Q15 math to
                        // `vcvt.f32.s32 s12, s12, #15` (em_iadc.c:427).
                        let unsigned = h1 & 1 != 0;
                        let size: u32 = if h2 & 0x80 != 0 { 32 } else { 16 };
                        let imm5 = (h2 & 0xf) << 1 | (h2 >> 5) & 1;
                        let fbits = size.saturating_sub(imm5);
                        let scale = (fbits as f64).exp2();
                        let slot = &mut self.fpregs[(d & 31) as usize];
                        if h1 & 0x4 == 0 {
                            // 0xA/0xB: fixed → float.
                            let int_val: f64 = match (size, unsigned) {
                                (32, false) => *slot as i32 as f64,
                                (32, true) => *slot as f64,
                                (_, false) => (*slot as u16 as i16) as f64,
                                (_, true) => (*slot as u16) as f64,
                            };
                            let v = (int_val / scale) as f32;
                            *slot = v.to_bits();
                        } else {
                            // 0xE/0xF: float → fixed, round toward zero,
                            // saturating to the container (FPToFixed).
                            let scaled = f32::from_bits(*slot) as f64 * scale;
                            *slot = match (size, unsigned) {
                                (32, false) => scaled as i32 as u32,
                                (32, true) => scaled as u32,
                                (_, false) => scaled as i16 as i32 as u32,
                                (_, true) => scaled as u16 as u32,
                            };
                        }
                    }
                    _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
                }
            }
            _ => self.break_cause = Some(BreakCause::Unimplemented(h1 | h2 << 16)),
        }
    }
}

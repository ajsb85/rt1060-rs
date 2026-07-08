//! Thumb opcode encoders for unit tests (port of rp2040js utils/assembler.ts
//! into Rust, same idea: keep instruction tests free of an external
//! assembler). Only encodings the core implements are provided; extend
//! alongside the executor.

pub fn adcs(rdn: u32, rm: u32) -> u16 {
    (0b0100000101 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn adds_imm3(rd: u32, rn: u32, imm3: u32) -> u16 {
    (0b0001110 << 9 | (imm3 & 7) << 6 | (rn & 7) << 3 | (rd & 7)) as u16
}

pub fn adds_imm8(rdn: u32, imm8: u32) -> u16 {
    (0b00110 << 11 | (rdn & 7) << 8 | (imm8 & 0xff)) as u16
}

pub fn adds_reg(rd: u32, rn: u32, rm: u32) -> u16 {
    (0b0001100 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rd & 7)) as u16
}

pub fn add_hi(rdn: u32, rm: u32) -> u16 {
    (0b01000100 << 8 | (rdn & 8) << 4 | (rm & 0xf) << 3 | (rdn & 7)) as u16
}

pub fn add_sp_imm7(imm: u32) -> u16 {
    (0b101100000 << 7 | (imm >> 2) & 0x7f) as u16
}

pub fn sub_sp_imm7(imm: u32) -> u16 {
    (0b101100001 << 7 | (imm >> 2) & 0x7f) as u16
}

pub fn add_rd_sp_imm8(rd: u32, imm: u32) -> u16 {
    (0b10101 << 11 | (rd & 7) << 8 | (imm >> 2) & 0xff) as u16
}

pub fn adr(rd: u32, imm: u32) -> u16 {
    (0b10100 << 11 | (rd & 7) << 8 | (imm >> 2) & 0xff) as u16
}

pub fn ands(rdn: u32, rm: u32) -> u16 {
    (0b0100000000 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn asrs_imm(rd: u32, rm: u32, imm5: u32) -> u16 {
    (0b00010 << 11 | (imm5 & 0x1f) << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn asrs_reg(rdn: u32, rm: u32) -> u16 {
    (0b0100000100 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

/// Conditional branch; `offset` is relative to PC+4 and must be even.
pub fn b_cond(cond: u32, offset: i32) -> u16 {
    (0b1101 << 12 | (cond & 0xf) << 8 | ((offset >> 1) as u32 & 0xff)) as u16
}

/// Unconditional branch; `offset` relative to PC+4, even, ±2KB.
pub fn b(offset: i32) -> u16 {
    (0b11100 << 11 | ((offset >> 1) as u32 & 0x7ff)) as u16
}

pub fn bics(rdn: u32, rm: u32) -> u16 {
    (0b0100001110 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn bkpt(imm8: u32) -> u16 {
    (0b10111110 << 8 | (imm8 & 0xff)) as u16
}

/// BL; returns (hw1, hw2). `offset` relative to PC+4.
pub fn bl(offset: i32) -> (u16, u16) {
    let imm = (offset >> 1) as u32;
    let s = (offset < 0) as u32;
    let imm11 = imm & 0x7ff;
    let imm10 = (imm >> 11) & 0x3ff;
    let i2 = (imm >> 21) & 1;
    let i1 = (imm >> 22) & 1;
    let j1 = i1 ^ s ^ 1; // J1 = NOT(I1 EOR S)
    let j2 = i2 ^ s ^ 1;
    let hw1 = (0b11110 << 11 | s << 10 | imm10) as u16;
    let hw2 = (0b11 << 14 | j1 << 13 | 1 << 12 | j2 << 11 | imm11) as u16;
    (hw1, hw2)
}

pub fn blx_reg(rm: u32) -> u16 {
    (0b010001111 << 7 | (rm & 0xf) << 3) as u16
}

pub fn bx(rm: u32) -> u16 {
    (0b010001110 << 7 | (rm & 0xf) << 3) as u16
}

/// CBZ/CBNZ; `offset` relative to PC+4, 0..=126, even.
pub fn cbz(rn: u32, offset: u32) -> u16 {
    (0b1011 << 12 | ((offset >> 6) & 1) << 9 | 1 << 8 | ((offset >> 1) & 0x1f) << 3 | (rn & 7))
        as u16
}

pub fn cbnz(rn: u32, offset: u32) -> u16 {
    cbz(rn, offset) | 1 << 11
}

pub fn cmn(rn: u32, rm: u32) -> u16 {
    (0b0100001011 << 6 | (rm & 7) << 3 | (rn & 7)) as u16
}

pub fn cmp_imm8(rn: u32, imm8: u32) -> u16 {
    (0b00101 << 11 | (rn & 7) << 8 | (imm8 & 0xff)) as u16
}

pub fn cmp_reg(rn: u32, rm: u32) -> u16 {
    (0b0100001010 << 6 | (rm & 7) << 3 | (rn & 7)) as u16
}

pub fn cmp_hi(rn: u32, rm: u32) -> u16 {
    (0b01000101 << 8 | (rn & 8) << 4 | (rm & 0xf) << 3 | (rn & 7)) as u16
}

pub fn cpsid_i() -> u16 {
    0xb672
}

pub fn cpsie_i() -> u16 {
    0xb662
}

pub fn eors(rdn: u32, rm: u32) -> u16 {
    (0b0100000001 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

/// IT block: `firstcond` + mask (mask encodes then/else pattern + stop bit).
pub fn it(firstcond: u32, mask: u32) -> u16 {
    (0b10111111 << 8 | (firstcond & 0xf) << 4 | (mask & 0xf)) as u16
}

pub fn ldmia(rn: u32, list: u32) -> u16 {
    (0b11001 << 11 | (rn & 7) << 8 | (list & 0xff)) as u16
}

pub fn ldr_imm(rt: u32, rn: u32, imm: u32) -> u16 {
    (0b01101 << 11 | ((imm >> 2) & 0x1f) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn ldr_lit(rt: u32, imm: u32) -> u16 {
    (0b01001 << 11 | (rt & 7) << 8 | (imm >> 2) & 0xff) as u16
}

pub fn ldr_sp(rt: u32, imm: u32) -> u16 {
    (0b10011 << 11 | (rt & 7) << 8 | (imm >> 2) & 0xff) as u16
}

pub fn ldr_reg(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101100 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn ldrb_imm(rt: u32, rn: u32, imm5: u32) -> u16 {
    (0b01111 << 11 | (imm5 & 0x1f) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn ldrb_reg(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101110 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn ldrh_imm(rt: u32, rn: u32, imm: u32) -> u16 {
    (0b10001 << 11 | ((imm >> 1) & 0x1f) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn ldrh_reg(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101101 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn ldrsb(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101011 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn ldrsh(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101111 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn lsls_imm(rd: u32, rm: u32, imm5: u32) -> u16 {
    // Top bits are 0b00000: the encoding really is just the low fields.
    ((imm5 & 0x1f) << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn lsls_reg(rdn: u32, rm: u32) -> u16 {
    (0b0100000010 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn lsrs_imm(rd: u32, rm: u32, imm5: u32) -> u16 {
    (0b00001 << 11 | (imm5 & 0x1f) << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn lsrs_reg(rdn: u32, rm: u32) -> u16 {
    (0b0100000011 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn movs_imm8(rd: u32, imm8: u32) -> u16 {
    (0b00100 << 11 | (rd & 7) << 8 | (imm8 & 0xff)) as u16
}

pub fn mov_hi(rd: u32, rm: u32) -> u16 {
    (0b01000110 << 8 | (rd & 8) << 4 | (rm & 0xf) << 3 | (rd & 7)) as u16
}

/// MRS; returns (hw1, hw2).
pub fn mrs(rd: u32, sysm: u32) -> (u16, u16) {
    (0xf3ef, (0x8000 | (rd & 0xf) << 8 | (sysm & 0xff)) as u16)
}

/// MSR; returns (hw1, hw2).
pub fn msr(sysm: u32, rn: u32) -> (u16, u16) {
    (
        (0xf380 | (rn & 0xf)) as u16,
        (0x8800 | (sysm & 0xff)) as u16,
    )
}

pub fn muls(rdm: u32, rn: u32) -> u16 {
    (0b0100001101 << 6 | (rn & 7) << 3 | (rdm & 7)) as u16
}

pub fn mvns(rd: u32, rm: u32) -> u16 {
    (0b0100001111 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn nop() -> u16 {
    0xbf00
}

pub fn orrs(rdn: u32, rm: u32) -> u16 {
    (0b0100001100 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn pop(pc: bool, list: u32) -> u16 {
    (0b1011110 << 9 | (pc as u32) << 8 | (list & 0xff)) as u16
}

pub fn push(lr: bool, list: u32) -> u16 {
    (0b1011010 << 9 | (lr as u32) << 8 | (list & 0xff)) as u16
}

pub fn rev(rd: u32, rm: u32) -> u16 {
    (0b1011101000 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn rev16(rd: u32, rm: u32) -> u16 {
    (0b1011101001 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn revsh(rd: u32, rm: u32) -> u16 {
    (0b1011101011 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn rors(rdn: u32, rm: u32) -> u16 {
    (0b0100000111 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn rsbs(rd: u32, rn: u32) -> u16 {
    (0b0100001001 << 6 | (rn & 7) << 3 | (rd & 7)) as u16
}

pub fn sbcs(rdn: u32, rm: u32) -> u16 {
    (0b0100000110 << 6 | (rm & 7) << 3 | (rdn & 7)) as u16
}

pub fn stmia(rn: u32, list: u32) -> u16 {
    (0b11000 << 11 | (rn & 7) << 8 | (list & 0xff)) as u16
}

pub fn str_imm(rt: u32, rn: u32, imm: u32) -> u16 {
    (0b01100 << 11 | ((imm >> 2) & 0x1f) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn str_sp(rt: u32, imm: u32) -> u16 {
    (0b10010 << 11 | (rt & 7) << 8 | (imm >> 2) & 0xff) as u16
}

pub fn str_reg(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101000 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn strb_imm(rt: u32, rn: u32, imm5: u32) -> u16 {
    (0b01110 << 11 | (imm5 & 0x1f) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn strb_reg(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101010 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn strh_imm(rt: u32, rn: u32, imm: u32) -> u16 {
    (0b10000 << 11 | ((imm >> 1) & 0x1f) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn strh_reg(rt: u32, rn: u32, rm: u32) -> u16 {
    (0b0101001 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rt & 7)) as u16
}

pub fn subs_imm3(rd: u32, rn: u32, imm3: u32) -> u16 {
    (0b0001111 << 9 | (imm3 & 7) << 6 | (rn & 7) << 3 | (rd & 7)) as u16
}

pub fn subs_imm8(rdn: u32, imm8: u32) -> u16 {
    (0b00111 << 11 | (rdn & 7) << 8 | (imm8 & 0xff)) as u16
}

pub fn subs_reg(rd: u32, rn: u32, rm: u32) -> u16 {
    (0b0001101 << 9 | (rm & 7) << 6 | (rn & 7) << 3 | (rd & 7)) as u16
}

pub fn svc(imm8: u32) -> u16 {
    (0b11011111 << 8 | (imm8 & 0xff)) as u16
}

pub fn sxtb(rd: u32, rm: u32) -> u16 {
    (0b1011001001 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn sxth(rd: u32, rm: u32) -> u16 {
    (0b1011001000 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn tst(rn: u32, rm: u32) -> u16 {
    (0b0100001000 << 6 | (rm & 7) << 3 | (rn & 7)) as u16
}

pub fn udf(imm8: u32) -> u16 {
    (0b11011110 << 8 | (imm8 & 0xff)) as u16
}

pub fn uxtb(rd: u32, rm: u32) -> u16 {
    (0b1011001011 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

pub fn uxth(rd: u32, rm: u32) -> u16 {
    (0b1011001010 << 6 | (rm & 7) << 3 | (rd & 7)) as u16
}

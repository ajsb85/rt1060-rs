//! Per-instruction unit tests, rp2040js-style: seed registers, place the
//! encoding in RAM, step once, assert registers AND all four flags.

use super::asm::*;
use super::*;

const RAM: u32 = 0x2000_0000;
const RAM_SIZE: usize = 0x4_0000;

/// Flat RAM at 0x2000_0000 — the CPU test fixture.
pub struct TestBus {
    pub ram: Vec<u8>,
}

impl TestBus {
    fn new() -> Self {
        TestBus {
            ram: vec![0; RAM_SIZE],
        }
    }

    fn off(addr: u32) -> usize {
        (addr - RAM) as usize
    }
}

impl Bus for TestBus {
    fn read8(&mut self, addr: u32) -> u8 {
        self.ram[Self::off(addr)]
    }
    fn read16(&mut self, addr: u32) -> u16 {
        let o = Self::off(addr);
        u16::from_le_bytes(self.ram[o..o + 2].try_into().unwrap())
    }
    fn read32(&mut self, addr: u32) -> u32 {
        let o = Self::off(addr);
        u32::from_le_bytes(self.ram[o..o + 4].try_into().unwrap())
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.ram[Self::off(addr)] = value;
    }
    fn write16(&mut self, addr: u32, value: u16) {
        let o = Self::off(addr);
        self.ram[o..o + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn write32(&mut self, addr: u32, value: u32) {
        let o = Self::off(addr);
        self.ram[o..o + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// Core with PC at RAM base and SP in the middle of RAM.
fn setup(code: &[u16]) -> (CortexM7, TestBus) {
    let mut cpu = CortexM7::new();
    let mut bus = TestBus::new();
    cpu.regs[PC] = RAM;
    cpu.regs[SP] = RAM + 0x2_0000;
    for (i, hw) in code.iter().enumerate() {
        bus.write16(RAM + 2 * i as u32, *hw);
    }
    (cpu, bus)
}

fn flags(cpu: &CortexM7) -> (bool, bool, bool, bool) {
    (cpu.n, cpu.z, cpu.c, cpu.v)
}

// --- ALU: add/sub families ------------------------------------------------

#[test]
fn adcs_overflow_case() {
    // rp2040js reference test: 0x7fffffff + 0 + C=0 -> no overflow.
    let (mut cpu, mut bus) = setup(&[adcs(3, 0)]);
    cpu.regs[3] = 0x7fff_ffff;
    cpu.regs[0] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 0x7fff_ffff);
    assert_eq!(flags(&cpu), (false, false, false, false));
}

#[test]
fn adcs_carry_in_wraps_to_zero() {
    // 0xffffffff + 0 + C=1 = 0 with carry out, no overflow.
    let (mut cpu, mut bus) = setup(&[adcs(1, 2)]);
    cpu.regs[1] = 0xffff_ffff;
    cpu.regs[2] = 0;
    cpu.c = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[1], 0);
    assert_eq!(flags(&cpu), (false, true, true, false));
}

#[test]
fn adcs_negative_overflow() {
    // 0x80000000 + 0x80000000 = 0: C=1 and V=1 (neg + neg = pos).
    let (mut cpu, mut bus) = setup(&[adcs(0, 1)]);
    cpu.regs[0] = 0x8000_0000;
    cpu.regs[1] = 0x8000_0000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert_eq!(flags(&cpu), (false, true, true, true));
}

#[test]
fn adds_imm8_carry_boundary() {
    // rp2040js had an off-by-one here: 0xffffffff + 1 = 0, C must be 1;
    // but 0xfffffffe + 1 = 0xffffffff must NOT set C.
    let (mut cpu, mut bus) = setup(&[adds_imm8(2, 1)]);
    cpu.regs[2] = 0xffff_fffe;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0xffff_ffff);
    assert_eq!(flags(&cpu), (true, false, false, false));

    let (mut cpu, mut bus) = setup(&[adds_imm8(2, 1)]);
    cpu.regs[2] = 0xffff_ffff;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0);
    assert_eq!(flags(&cpu), (false, true, true, false));
}

#[test]
fn adds_reg_and_imm3() {
    let (mut cpu, mut bus) = setup(&[adds_reg(0, 1, 2), adds_imm3(3, 0, 7)]);
    cpu.regs[1] = 5;
    cpu.regs[2] = 7;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 12);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 19);
    assert_eq!(flags(&cpu), (false, false, false, false));
}

#[test]
fn subs_borrow_semantics() {
    // ARM subtract: C is NOT-borrow. 5 - 7 -> C=0; 7 - 5 -> C=1.
    let (mut cpu, mut bus) = setup(&[subs_reg(0, 1, 2)]);
    cpu.regs[1] = 5;
    cpu.regs[2] = 7;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], (-2i32) as u32);
    assert_eq!(flags(&cpu), (true, false, false, false));

    let (mut cpu, mut bus) = setup(&[subs_reg(0, 1, 2)]);
    cpu.regs[1] = 7;
    cpu.regs[2] = 5;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 2);
    assert_eq!(flags(&cpu), (false, false, true, false));
}

#[test]
fn subs_overflow() {
    // 0x80000000 - 1 = 0x7fffffff: V=1 (neg - pos = pos), C=1.
    let (mut cpu, mut bus) = setup(&[subs_imm8(0, 1)]);
    cpu.regs[0] = 0x8000_0000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x7fff_ffff);
    assert_eq!(flags(&cpu), (false, false, true, true));
}

#[test]
fn sbcs_with_borrow() {
    // SBC: Rd = Rn - Rm - !C. With C=0: 10 - 3 - 1 = 6, C=1 out.
    let (mut cpu, mut bus) = setup(&[sbcs(0, 1)]);
    cpu.regs[0] = 10;
    cpu.regs[1] = 3;
    cpu.c = false;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 6);
    assert_eq!(flags(&cpu), (false, false, true, false));
}

#[test]
fn rsbs_negate() {
    let (mut cpu, mut bus) = setup(&[rsbs(0, 1)]);
    cpu.regs[1] = 5;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], (-5i32) as u32);
    assert_eq!(flags(&cpu), (true, false, false, false));

    // RSBS of 0: result 0, Z=1, C=1 (0 - 0 no borrow).
    let (mut cpu, mut bus) = setup(&[rsbs(0, 1)]);
    cpu.regs[1] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert_eq!(flags(&cpu), (false, true, true, false));
}

#[test]
fn cmp_and_cmn() {
    let (mut cpu, mut bus) = setup(&[cmp_imm8(0, 66), cmp_reg(1, 2), cmn(3, 4)]);
    cpu.regs[0] = 66; // equal -> Z=1, C=1
    cpu.step(&mut bus);
    assert_eq!(flags(&cpu), (false, true, true, false));
    cpu.regs[1] = 1;
    cpu.regs[2] = 2; // 1 - 2 -> negative, borrow
    cpu.step(&mut bus);
    assert_eq!(flags(&cpu), (true, false, false, false));
    cpu.regs[3] = 1;
    cpu.regs[4] = (-1i32) as u32; // CMN: 1 + (-1) = 0 -> Z=1, C=1
    cpu.step(&mut bus);
    assert_eq!(flags(&cpu), (false, true, true, false));
}

// --- ALU: logic and shifts --------------------------------------------------

#[test]
fn logic_ops_preserve_c_and_v() {
    let (mut cpu, mut bus) = setup(&[ands(0, 1), orrs(2, 3), eors(4, 5), bics(6, 7)]);
    cpu.c = true;
    cpu.v = true;
    cpu.regs[0] = 0xf0f0;
    cpu.regs[1] = 0x0ff0;
    cpu.regs[2] = 0x1;
    cpu.regs[3] = 0x8000_0000;
    cpu.regs[4] = 0xffff_ffff;
    cpu.regs[5] = 0xffff_ffff;
    cpu.regs[6] = 0xff;
    cpu.regs[7] = 0x0f;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x0f0);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0x8000_0001);
    assert!(cpu.n);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[4], 0);
    assert!(cpu.z);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[6], 0xf0);
    // C and V untouched by logic ops.
    assert!(cpu.c);
    assert!(cpu.v);
}

#[test]
fn lsls_imm_and_carry_out() {
    let (mut cpu, mut bus) = setup(&[lsls_imm(0, 1, 1)]);
    cpu.regs[1] = 0x8000_0001;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 2);
    assert_eq!(flags(&cpu), (false, false, true, false));
}

#[test]
fn lsls_imm5_zero_is_movs_keeps_carry() {
    let (mut cpu, mut bus) = setup(&[lsls_imm(0, 1, 0)]);
    cpu.regs[1] = 0x8000_0000;
    cpu.c = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x8000_0000);
    // MOV-shift by 0: C unchanged.
    assert_eq!(flags(&cpu), (true, false, true, false));
}

#[test]
fn lsrs_imm5_zero_means_shift_32() {
    let (mut cpu, mut bus) = setup(&[lsrs_imm(0, 1, 0)]);
    cpu.regs[1] = 0x8000_0000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert_eq!(flags(&cpu), (false, true, true, false));
}

#[test]
fn asrs_imm5_zero_sign_fills() {
    let (mut cpu, mut bus) = setup(&[asrs_imm(0, 1, 0)]);
    cpu.regs[1] = 0x8000_0000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_ffff);
    assert_eq!(flags(&cpu), (true, false, true, false));
}

#[test]
fn shift_by_register_large_amounts() {
    // LSL by 33 via register: result 0, C=0.
    let (mut cpu, mut bus) = setup(&[lsls_reg(0, 1)]);
    cpu.regs[0] = 0xffff_ffff;
    cpu.regs[1] = 33;
    cpu.c = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert_eq!(flags(&cpu), (false, true, false, false));

    // LSR by exactly 32: C = old bit31.
    let (mut cpu, mut bus) = setup(&[lsrs_reg(0, 1)]);
    cpu.regs[0] = 0x8000_0000;
    cpu.regs[1] = 32;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert_eq!(flags(&cpu), (false, true, true, false));

    // Shift by 0 via register: everything unchanged.
    let (mut cpu, mut bus) = setup(&[lsrs_reg(0, 1)]);
    cpu.regs[0] = 0x1234;
    cpu.regs[1] = 0;
    cpu.c = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x1234);
    assert!(cpu.c);
}

#[test]
fn rors_rotate_and_flags() {
    let (mut cpu, mut bus) = setup(&[rors(0, 1)]);
    cpu.regs[0] = 0x0000_0001;
    cpu.regs[1] = 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x8000_0000);
    assert_eq!(flags(&cpu), (true, false, true, false));
}

#[test]
fn muls_leaves_c_v_alone() {
    let (mut cpu, mut bus) = setup(&[muls(0, 1)]);
    cpu.regs[0] = 3;
    cpu.regs[1] = 5;
    cpu.c = true;
    cpu.v = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 15);
    assert_eq!(flags(&cpu), (false, false, true, true));
}

#[test]
fn mvns_and_tst() {
    let (mut cpu, mut bus) = setup(&[mvns(0, 1), tst(2, 3)]);
    cpu.regs[1] = 0x0000_ffff;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_0000);
    assert!(cpu.n);
    cpu.regs[2] = 0xf0;
    cpu.regs[3] = 0x0f;
    cpu.step(&mut bus);
    assert!(cpu.z);
}

// --- MOV / hi-register ops ---------------------------------------------------

#[test]
fn movs_imm8_flags() {
    let (mut cpu, mut bus) = setup(&[movs_imm8(5, 0)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[5], 0);
    assert_eq!(flags(&cpu), (false, true, false, false));
}

#[test]
fn mov_hi_no_flags_and_pc_dest() {
    let (mut cpu, mut bus) = setup(&[mov_hi(8, 1)]);
    cpu.regs[1] = 0x8000_0000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[8], 0x8000_0000);
    // hi-reg MOV never sets flags.
    assert_eq!(flags(&cpu), (false, false, false, false));

    // MOV PC, Rm branches (bit 0 cleared).
    let (mut cpu, mut bus) = setup(&[mov_hi(15, 2)]);
    cpu.regs[2] = RAM + 0x101;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x100);
}

#[test]
fn add_hi_with_pc_source() {
    // ADD r0, pc: PC reads as instruction address + 4.
    let (mut cpu, mut bus) = setup(&[add_hi(0, 15)]);
    cpu.regs[0] = 8;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], RAM + 4 + 8);
}

// --- Loads and stores ---------------------------------------------------------

#[test]
fn ldr_str_word_imm_and_reg() {
    let (mut cpu, mut bus) = setup(&[
        str_imm(0, 1, 8),
        ldr_imm(2, 1, 8),
        str_reg(0, 1, 3),
        ldr_reg(4, 1, 3),
    ]);
    cpu.regs[0] = 0xdead_beef;
    cpu.regs[1] = RAM + 0x1000;
    cpu.regs[3] = 0x20;
    cpu.step(&mut bus);
    assert_eq!(bus.read32(RAM + 0x1008), 0xdead_beef);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0xdead_beef);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[4], 0xdead_beef);
}

#[test]
fn byte_and_half_accesses() {
    let (mut cpu, mut bus) = setup(&[
        strb_imm(0, 1, 1),
        ldrb_imm(2, 1, 1),
        strh_imm(0, 1, 4),
        ldrh_imm(3, 1, 4),
    ]);
    cpu.regs[0] = 0x1234_56ab;
    cpu.regs[1] = RAM + 0x2000;
    cpu.step(&mut bus);
    assert_eq!(bus.read8(RAM + 0x2001), 0xab);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0xab);
    cpu.step(&mut bus);
    assert_eq!(bus.read16(RAM + 0x2004), 0x56ab);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 0x56ab);
}

#[test]
fn signed_loads_extend() {
    let (mut cpu, mut bus) = setup(&[ldrsb(2, 0, 1), ldrsh(3, 0, 4)]);
    cpu.regs[0] = RAM + 0x3000;
    cpu.regs[1] = 0;
    cpu.regs[4] = 2;
    bus.write8(RAM + 0x3000, 0x80);
    bus.write16(RAM + 0x3002, 0x8001);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0xffff_ff80);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 0xffff_8001);
}

#[test]
fn ldr_literal_aligns_pc() {
    // Place literal at RAM+8; instruction at RAM+2 (odd halfword) so that
    // PC+4 = RAM+6 must align down to RAM+4... offset 4 -> RAM+8.
    let (mut cpu, mut bus) = setup(&[nop(), ldr_lit(0, 4)]);
    bus.write32(RAM + 8, 0x1234_5678);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x1234_5678);
}

#[test]
fn sp_relative_and_adr() {
    let (mut cpu, mut bus) = setup(&[str_sp(0, 8), ldr_sp(1, 8), adr(2, 4), add_rd_sp_imm8(3, 16)]);
    let sp = cpu.regs[SP];
    cpu.regs[0] = 0xc0ff_ee00;
    cpu.step(&mut bus);
    assert_eq!(bus.read32(sp + 8), 0xc0ff_ee00);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[1], 0xc0ff_ee00);
    cpu.step(&mut bus);
    // ADR at RAM+4: align(PC+4) = RAM+8, plus imm 4.
    assert_eq!(cpu.regs[2], RAM + 8 + 4);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], sp + 16);
}

#[test]
fn push_pop_roundtrip_with_pc() {
    let (mut cpu, mut bus) = setup(&[push(true, 0b0000_0101), pop(true, 0b0000_0101)]);
    let sp0 = cpu.regs[SP];
    cpu.regs[0] = 11;
    cpu.regs[2] = 22;
    cpu.regs[LR] = (RAM + 0x40) | 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[SP], sp0 - 12);
    // Lowest register at lowest address; LR highest.
    assert_eq!(bus.read32(sp0 - 12), 11);
    assert_eq!(bus.read32(sp0 - 8), 22);
    assert_eq!(bus.read32(sp0 - 4), (RAM + 0x40) | 1);
    cpu.regs[0] = 0;
    cpu.regs[2] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[SP], sp0);
    assert_eq!(cpu.regs[0], 11);
    assert_eq!(cpu.regs[2], 22);
    // POP {pc} interworks: bit 0 stripped.
    assert_eq!(cpu.regs[PC], RAM + 0x40);
}

#[test]
fn stm_ldm_writeback() {
    // Base register outside the list on both sides so writeback applies.
    let (mut cpu, mut bus) = setup(&[stmia(0, 0b1100), ldmia(1, 0b1100)]);
    cpu.regs[0] = RAM + 0x4000;
    cpu.regs[2] = 0xaa;
    cpu.regs[3] = 0xbb;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], RAM + 0x4008); // writeback
    assert_eq!(bus.read32(RAM + 0x4000), 0xaa);
    assert_eq!(bus.read32(RAM + 0x4004), 0xbb);
    cpu.regs[1] = RAM + 0x4000;
    cpu.regs[2] = 0;
    cpu.regs[3] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[1], RAM + 0x4008);
    assert_eq!(cpu.regs[2], 0xaa);
    assert_eq!(cpu.regs[3], 0xbb);
}

#[test]
fn ldm_base_in_list_suppresses_writeback() {
    let (mut cpu, mut bus) = setup(&[ldmia(1, 0b0010)]);
    bus.write32(RAM + 0x4000, 0x777);
    cpu.regs[1] = RAM + 0x4000;
    cpu.step(&mut bus);
    // Rn in the list: loaded value wins, no writeback.
    assert_eq!(cpu.regs[1], 0x777);
}

// --- Extends / byte-reversal ---------------------------------------------------

#[test]
fn extend_ops() {
    let (mut cpu, mut bus) = setup(&[sxtb(0, 1), sxth(2, 1), uxtb(3, 1), uxth(4, 1)]);
    cpu.regs[1] = 0x0001_8281;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_ff81);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0xffff_8281);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 0x81);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[4], 0x8281);
}

#[test]
fn rev_family() {
    let (mut cpu, mut bus) = setup(&[rev(0, 1), rev16(2, 1), revsh(3, 1)]);
    cpu.regs[1] = 0x1122_3344;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x4433_2211);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0x2211_4433);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 0x0000_4433);
}

// --- Branches ------------------------------------------------------------------

#[test]
fn b_cond_taken_and_not_taken() {
    // BEQ +8 with Z=0: falls through to next instruction.
    let (mut cpu, mut bus) = setup(&[b_cond(0, 8)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 2);
    // With Z=1: branches to pc4 + 8.
    let (mut cpu, mut bus) = setup(&[b_cond(0, 8)]);
    cpu.z = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 4 + 8);
}

#[test]
fn b_uncond_backwards() {
    let (mut cpu, mut bus) = setup(&[nop(), nop(), b(-8)]);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM); // (RAM+4)+4-8
}

#[test]
fn bl_sets_lr_and_branches() {
    let (mut cpu, mut bus) = setup(&[]);
    let (h1, h2) = bl(0x10);
    bus.write16(RAM, h1);
    bus.write16(RAM + 2, h2);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[LR], (RAM + 4) | 1);
    assert_eq!(cpu.regs[PC], RAM + 4 + 0x10);
}

#[test]
fn bl_negative_offset() {
    let (mut cpu, mut bus) = setup(&[]);
    let start = RAM + 0x100;
    let (h1, h2) = bl(-0x20);
    bus.write16(start, h1);
    bus.write16(start + 2, h2);
    cpu.regs[PC] = start;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], start + 4 - 0x20);
}

#[test]
fn bx_and_blx_reg() {
    let (mut cpu, mut bus) = setup(&[bx(1)]);
    cpu.regs[1] = (RAM + 0x80) | 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x80);
    assert_eq!(cpu.regs[LR], 0); // BX does not touch LR

    let (mut cpu, mut bus) = setup(&[blx_reg(2)]);
    cpu.regs[2] = (RAM + 0x80) | 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x80);
    assert_eq!(cpu.regs[LR], (RAM + 2) | 1);
}

#[test]
fn cbz_cbnz() {
    let (mut cpu, mut bus) = setup(&[cbz(0, 8)]);
    cpu.regs[0] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 4 + 8);

    let (mut cpu, mut bus) = setup(&[cbz(0, 8)]);
    cpu.regs[0] = 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 2);

    let (mut cpu, mut bus) = setup(&[cbnz(0, 8)]);
    cpu.regs[0] = 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 4 + 8);
}

// --- IT blocks -------------------------------------------------------------------

#[test]
fn it_block_then_else() {
    // ITE EQ: first instr executes if Z, second if !Z. Members don't set
    // flags. firstcond=EQ(0), mask for ITE = cond[0]^1... T then E with
    // firstcond even: mask = 0b0100 | stop -> ITE EQ = 0xbf0c.
    let (mut cpu, mut bus) = setup(&[
        it(0, 0b1100), // ITE EQ
        movs_imm8(0, 1),
        movs_imm8(0, 2),
        nop(),
    ]);
    cpu.z = true;
    cpu.step(&mut bus); // IT
    cpu.step(&mut bus); // MOVS r0,#1 (executes)
    assert_eq!(cpu.regs[0], 1);
    // flag untouched inside IT block despite "movs" encoding:
    assert!(cpu.z);
    cpu.step(&mut bus); // MOVS r0,#2 (skipped)
    assert_eq!(cpu.regs[0], 1);
    // Three 2-byte instructions retired: PC sits on the NOP.
    assert_eq!(cpu.regs[PC], RAM + 6);

    let (mut cpu, mut bus) = setup(&[it(0, 0b1100), movs_imm8(0, 1), movs_imm8(0, 2), nop()]);
    cpu.z = false;
    cpu.step(&mut bus);
    cpu.step(&mut bus); // skipped
    assert_eq!(cpu.regs[0], 0);
    cpu.step(&mut bus); // executes
    assert_eq!(cpu.regs[0], 2);
}

#[test]
fn it_block_cmp_still_sets_flags() {
    // IT GT; CMPGT inside an IT block still updates flags.
    // GT = cond 12 (0b1100); IT GT single: mask = 0b1000.
    let (mut cpu, mut bus) = setup(&[it(12, 0b1000), cmp_imm8(0, 5)]);
    cpu.regs[0] = 9; // GT requires Z=0 && N==V: true initially
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    // 9 - 5 = 4: C=1 (no borrow), N=0.
    assert!(cpu.c);
    assert!(!cpu.n);
    assert!(!cpu.z);
}

// --- System ------------------------------------------------------------------------

#[test]
fn mrs_msr_msp_psp_control() {
    let (mut cpu, mut bus) = setup(&[]);
    let (h1, h2) = msr(8, 0); // MSR MSP, r0
    bus.write16(RAM, h1);
    bus.write16(RAM + 2, h2);
    let (h1, h2) = mrs(1, 8); // MRS r1, MSP
    bus.write16(RAM + 4, h1);
    bus.write16(RAM + 6, h2);
    let (h1, h2) = msr(9, 2); // MSR PSP, r2
    bus.write16(RAM + 8, h1);
    bus.write16(RAM + 10, h2);
    let (h1, h2) = mrs(3, 9); // MRS r3, PSP
    bus.write16(RAM + 12, h1);
    bus.write16(RAM + 14, h2);
    cpu.regs[0] = 0x2003_0000;
    cpu.regs[2] = 0x2002_0000;
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[1], 0x2003_0000);
    assert_eq!(cpu.regs[SP], 0x2003_0000); // MSP active in thread mode
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 0x2002_0000);
    assert_eq!(cpu.regs[SP], 0x2003_0000); // active SP still MSP
}

#[test]
fn mrs_reads_ipsr_exception_number() {
    // `mrs Rd, IPSR` (SYSm 5) must yield the active exception number, not 0 —
    // Zephyr's `_isr_wrapper` indexes the software ISR table with it, so 0
    // sends every external interrupt through a garbage handler.
    let (mut cpu, mut bus) = setup(&[]);
    let (h1, h2) = mrs(0, 5); // MRS r0, IPSR
    bus.write16(RAM, h1);
    bus.write16(RAM + 2, h2);
    cpu.ipsr = 84; // in exception 84 (external IRQ 68)
    cpu.n = true; // APSR flags must NOT leak into IPSR
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 84);

    // `mrs Rd, xPSR` (SYSm 3) combines APSR + IPSR.
    let (mut cpu, mut bus) = setup(&[]);
    let (h1, h2) = mrs(1, 3);
    bus.write16(RAM, h1);
    bus.write16(RAM + 2, h2);
    cpu.ipsr = 20;
    cpu.n = true; // APSR.N (bit 31)
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[1], (1 << 31) | 20);
}

#[test]
fn cps_and_primask_via_mrs() {
    let (mut cpu, mut bus) = setup(&[cpsid_i(), cpsie_i()]);
    cpu.step(&mut bus);
    assert!(cpu.primask);
    cpu.step(&mut bus);
    assert!(!cpu.primask);
}

#[test]
fn bkpt_and_udf_raise_break() {
    let (mut cpu, mut bus) = setup(&[bkpt(0x42)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.break_cause, Some(BreakCause::Bkpt(0x42)));

    let (mut cpu, mut bus) = setup(&[udf(0x07)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.break_cause, Some(BreakCause::Udf(0x07)));
}

#[test]
fn add_sub_sp_imm7() {
    let (mut cpu, mut bus) = setup(&[sub_sp_imm7(0x40), add_sp_imm7(0x10)]);
    let sp0 = cpu.regs[SP];
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[SP], sp0 - 0x40);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[SP], sp0 - 0x30);
}

#[test]
fn reset_loads_vector_table() {
    let (mut cpu, mut bus) = setup(&[]);
    cpu.vtor = RAM + 0x1000;
    bus.write32(RAM + 0x1000, 0x2008_2000); // initial MSP
    bus.write32(RAM + 0x1004, (RAM + 0x200) | 1); // reset vector, thumb
    cpu.reset(&mut bus);
    assert_eq!(cpu.regs[SP], 0x2008_2000);
    assert_eq!(cpu.regs[PC], RAM + 0x200);
}

#[test]
fn xpsr_packing() {
    let mut cpu = CortexM7::new();
    cpu.n = true;
    cpu.c = true;
    // xPSR: N at bit 31, C at bit 29, T at bit 24. (rp2040js had these at
    // the wrong bit positions — this test pins the correct ones.)
    assert_eq!(cpu.xpsr(), 0xa100_0000);
}

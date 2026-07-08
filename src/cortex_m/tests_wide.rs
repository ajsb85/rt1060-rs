//! Wide (32-bit Thumb-2) instruction tests. Every encoding literal below is
//! taken verbatim from GNU `as` output (`-march=armv8-m.main`), so the
//! decoder is tested against the real assembler, not our own encoders.

use super::tests::TestBus;
use super::*;

const RAM: u32 = 0x2000_0000;

fn setup_w(words: &[(u16, u16)]) -> (CortexM7, TestBus) {
    let mut cpu = CortexM7::new();
    let mut bus = TestBus {
        ram: vec![0; 0x4_0000],
    };
    cpu.regs[PC] = RAM;
    cpu.regs[SP] = RAM + 0x2_0000;
    for (i, (h1, h2)) in words.iter().enumerate() {
        bus.write16(RAM + 4 * i as u32, *h1);
        bus.write16(RAM + 4 * i as u32 + 2, *h2);
    }
    (cpu, bus)
}

// --- Data processing, shifted register --------------------------------------

#[test]
fn add_w_and_flags_variant() {
    // eb01 0002  add.w r0, r1, r2 ; eb11 0002  adds.w r0, r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xeb01, 0x0002)]);
    cpu.regs[1] = 7;
    cpu.regs[2] = 5;
    cpu.c = true; // must not affect ADD, must not be clobbered (no S)
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 12);
    assert!(cpu.c && !cpu.z && !cpu.n);

    let (mut cpu, mut bus) = setup_w(&[(0xeb11, 0x0002)]);
    cpu.regs[1] = 1;
    cpu.regs[2] = (-1i32) as u32;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert!(cpu.z && cpu.c && !cpu.v);
}

#[test]
fn add_w_shifted_operand() {
    // eb01 00c2  add.w r0, r1, r2, lsl #3
    let (mut cpu, mut bus) = setup_w(&[(0xeb01, 0x00c2)]);
    cpu.regs[1] = 1;
    cpu.regs[2] = 4;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 1 + (4 << 3));
}

#[test]
fn sub_w_shifted_and_orrs_asr() {
    // eba1 1052  sub.w r0, r1, r2, lsr #5
    let (mut cpu, mut bus) = setup_w(&[(0xeba1, 0x1052)]);
    cpu.regs[1] = 0x100;
    cpu.regs[2] = 0x20 << 5;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x100 - 0x20);

    // ea51 10e2  orrs.w r0, r1, r2, asr #7 — C comes from the shifter.
    let (mut cpu, mut bus) = setup_w(&[(0xea51, 0x10e2)]);
    cpu.regs[1] = 0;
    cpu.regs[2] = 0x8000_0040; // bit 6 shifts out last -> C=1
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], ((0x8000_0040u32 as i32) >> 7) as u32);
    assert!(cpu.c && cpu.n);
}

#[test]
fn mov_w_rrx_and_ror() {
    // ea4f 0031  mov.w r0, r1, rrx
    let (mut cpu, mut bus) = setup_w(&[(0xea4f, 0x0031)]);
    cpu.regs[1] = 0x3;
    cpu.c = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x8000_0001);
    assert!(cpu.c); // no S: flags untouched

    // ea5f 0071  movs.w r0, r1, ror #1
    let (mut cpu, mut bus) = setup_w(&[(0xea5f, 0x0071)]);
    cpu.regs[1] = 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x8000_0000);
    assert!(cpu.c && cpu.n);
}

#[test]
fn mvn_w_teq_cmn_w() {
    // ea6f 0001  mvn.w r0, r1
    let (mut cpu, mut bus) = setup_w(&[(0xea6f, 0x0001)]);
    cpu.regs[1] = 0x00ff_00ff;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xff00_ff00);

    // ea91 0f02  teq r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xea91, 0x0f02)]);
    cpu.regs[1] = 0x55;
    cpu.regs[2] = 0x55;
    cpu.step(&mut bus);
    assert!(cpu.z);
    assert_eq!(cpu.regs[15], RAM + 4); // no register written

    // eb11 0f02  cmn.w r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xeb11, 0x0f02)]);
    cpu.regs[1] = 1;
    cpu.regs[2] = (-1i32) as u32;
    cpu.step(&mut bus);
    assert!(cpu.z && cpu.c);
}

#[test]
fn adc_sbc_wide() {
    // eb41 0002  adc.w r0, r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xeb41, 0x0002)]);
    cpu.regs[1] = 5;
    cpu.regs[2] = 5;
    cpu.c = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 11);

    // eb71 0002  sbcs.w r0, r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xeb71, 0x0002)]);
    cpu.regs[1] = 10;
    cpu.regs[2] = 3;
    cpu.c = true; // no borrow
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 7);
    assert!(cpu.c);
}

// --- Data processing, modified immediate -------------------------------------

#[test]
fn mod_imm_patterns() {
    // f44f 3090  mov.w r0, #0x12000  (rotated immediate)
    let (mut cpu, mut bus) = setup_w(&[(0xf44f, 0x3090)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x12000);

    // f015 24ff  ands.w r4, r5, #0xff00ff00  (splat pattern 10)
    let (mut cpu, mut bus) = setup_w(&[(0xf015, 0x24ff)]);
    cpu.regs[5] = 0x1234_5678;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[4], 0x1200_5600);

    // f041 00ab  orr.w r0, r1, #0xab  (plain byte)
    let (mut cpu, mut bus) = setup_w(&[(0xf041, 0x00ab)]);
    cpu.regs[1] = 0x100;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x1ab);

    // f07f 4000  mvns.w r0, #0x80000000 — expansion carry-out sets C.
    let (mut cpu, mut bus) = setup_w(&[(0xf07f, 0x4000)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x7fff_ffff);
    assert!(cpu.c && !cpu.n);
}

#[test]
fn subs_w_imm_and_cmp_w_imm() {
    // f1b1 0011  subs.w r0, r1, #17
    let (mut cpu, mut bus) = setup_w(&[(0xf1b1, 0x0011)]);
    cpu.regs[1] = 17;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert!(cpu.z && cpu.c);

    // f5b1 4fff  cmp.w r1, #0x7f80
    let (mut cpu, mut bus) = setup_w(&[(0xf5b1, 0x4fff)]);
    cpu.regs[1] = 0x7f80;
    cpu.step(&mut bus);
    assert!(cpu.z && cpu.c);

    // f011 0f01  tst.w r1, #1
    let (mut cpu, mut bus) = setup_w(&[(0xf011, 0x0f01)]);
    cpu.regs[1] = 2;
    cpu.step(&mut bus);
    assert!(cpu.z);

    // f1c1 0000  rsb r0, r1, #0
    let (mut cpu, mut bus) = setup_w(&[(0xf1c1, 0x0000)]);
    cpu.regs[1] = 7;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], (-7i32) as u32);
}

// --- Plain immediates: MOVW/MOVT/ADDW/SUBW/bitfield ---------------------------

#[test]
fn movw_movt_pair() {
    // f241 2034  movw r0, #0x1234 ; f2c5 6078  movt r0, #0x5678
    let (mut cpu, mut bus) = setup_w(&[(0xf241, 0x2034), (0xf2c5, 0x6078)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x1234);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x5678_1234);
}

#[test]
fn addw_subw() {
    // f601 70ff  addw r0, r1, #0xfff
    let (mut cpu, mut bus) = setup_w(&[(0xf601, 0x70ff)]);
    cpu.regs[1] = 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x1000);

    // f2ad 1023  subw r0, sp, #0x123
    let (mut cpu, mut bus) = setup_w(&[(0xf2ad, 0x1023)]);
    let sp = cpu.regs[SP];
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], sp - 0x123);
}

#[test]
fn bitfield_ops() {
    // f3c1 1007  ubfx r0, r1, #4, #8
    let (mut cpu, mut bus) = setup_w(&[(0xf3c1, 0x1007)]);
    cpu.regs[1] = 0x0000_a5f0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x5f);

    // f341 1007  sbfx r0, r1, #4, #8
    let (mut cpu, mut bus) = setup_w(&[(0xf341, 0x1007)]);
    cpu.regs[1] = 0x0000_8f00; // field = 0xf0 -> sign-extends
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_fff0);

    // f361 100b  bfi r0, r1, #4, #8
    let (mut cpu, mut bus) = setup_w(&[(0xf361, 0x100b)]);
    cpu.regs[0] = 0xffff_ffff;
    cpu.regs[1] = 0x0000_00ab;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_fabf);

    // f36f 100b  bfc r0, #4, #8
    let (mut cpu, mut bus) = setup_w(&[(0xf36f, 0x100b)]);
    cpu.regs[0] = 0xffff_ffff;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_f00f);
}

// --- Loads/stores --------------------------------------------------------------

#[test]
fn ldr_str_imm12_and_negative_imm8() {
    // f8c1 0123  str.w r0, [r1, #0x123] ; f8d1 0123  ldr.w r0, [r1, #0x123]
    let (mut cpu, mut bus) = setup_w(&[(0xf8c1, 0x0123), (0xf8d1, 0x0123)]);
    cpu.regs[0] = 0xfeed_f00d;
    cpu.regs[1] = RAM + 0x1000;
    cpu.step(&mut bus);
    assert_eq!(bus.read32(RAM + 0x1123), 0xfeed_f00d);
    cpu.regs[0] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xfeed_f00d);

    // f851 0c04  ldr.w r0, [r1, #-4]
    let (mut cpu, mut bus) = setup_w(&[(0xf851, 0x0c04)]);
    bus.write32(RAM + 0x0ffc, 0x4242);
    cpu.regs[1] = RAM + 0x1000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x4242);
}

#[test]
fn ldr_post_index_and_str_pre_index_writeback() {
    // f851 0b04  ldr.w r0, [r1], #4
    let (mut cpu, mut bus) = setup_w(&[(0xf851, 0x0b04)]);
    bus.write32(RAM + 0x1000, 77);
    cpu.regs[1] = RAM + 0x1000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 77);
    assert_eq!(cpu.regs[1], RAM + 0x1004);

    // f841 0d08  str.w r0, [r1, #-8]!
    let (mut cpu, mut bus) = setup_w(&[(0xf841, 0x0d08)]);
    cpu.regs[0] = 88;
    cpu.regs[1] = RAM + 0x1000;
    cpu.step(&mut bus);
    assert_eq!(bus.read32(RAM + 0x0ff8), 88);
    assert_eq!(cpu.regs[1], RAM + 0x0ff8);
}

#[test]
fn wide_byte_half_signed() {
    // f893/f993/f8b3/f9b3 2040: ldrb/ldrsb/ldrh/ldrsh.w r2, [r3, #0x40]
    let (mut cpu, mut bus) = setup_w(&[
        (0xf893, 0x2040),
        (0xf993, 0x2040),
        (0xf8b3, 0x2040),
        (0xf9b3, 0x2040),
    ]);
    cpu.regs[3] = RAM + 0x2000;
    bus.write16(RAM + 0x2040, 0x8091);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0x91);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0xffff_ff91);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0x8091);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[2], 0xffff_8091);
}

#[test]
fn ldr_register_shifted_and_pc_pop() {
    // f851 0022  ldr.w r0, [r1, r2, lsl #2]
    let (mut cpu, mut bus) = setup_w(&[(0xf851, 0x0022)]);
    cpu.regs[1] = RAM + 0x3000;
    cpu.regs[2] = 4;
    bus.write32(RAM + 0x3010, 99);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 99);

    // f85d fb04  ldr.w pc, [sp], #4  (pop.w {pc})
    let (mut cpu, mut bus) = setup_w(&[(0xf85d, 0xfb04)]);
    let sp = cpu.regs[SP];
    bus.write32(sp, (RAM + 0x500) | 1);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x500);
    assert_eq!(cpu.regs[SP], sp + 4);
}

#[test]
fn ldrd_strd_forms() {
    // e9c2 0102  strd r0, r1, [r2, #8] ; e9d2 0102  ldrd r0, r1, [r2, #8]
    let (mut cpu, mut bus) = setup_w(&[(0xe9c2, 0x0102), (0xe9d2, 0x0102)]);
    cpu.regs[0] = 0x1111;
    cpu.regs[1] = 0x2222;
    cpu.regs[2] = RAM + 0x4000;
    cpu.step(&mut bus);
    assert_eq!(bus.read32(RAM + 0x4008), 0x1111);
    assert_eq!(bus.read32(RAM + 0x400c), 0x2222);
    cpu.regs[0] = 0;
    cpu.regs[1] = 0;
    cpu.step(&mut bus);
    assert_eq!((cpu.regs[0], cpu.regs[1]), (0x1111, 0x2222));

    // e8f2 0102  ldrd r0, r1, [r2], #8  (post-index)
    let (mut cpu, mut bus) = setup_w(&[(0xe8f2, 0x0102)]);
    cpu.regs[2] = RAM + 0x4000;
    bus.write32(RAM + 0x4000, 5);
    bus.write32(RAM + 0x4004, 6);
    cpu.step(&mut bus);
    assert_eq!((cpu.regs[0], cpu.regs[1]), (5, 6));
    assert_eq!(cpu.regs[2], RAM + 0x4008);

    // e962 0104  strd r0, r1, [r2, #-16]!  (pre-index writeback)
    let (mut cpu, mut bus) = setup_w(&[(0xe962, 0x0104)]);
    cpu.regs[0] = 7;
    cpu.regs[1] = 8;
    cpu.regs[2] = RAM + 0x4020;
    cpu.step(&mut bus);
    assert_eq!(bus.read32(RAM + 0x4010), 7);
    assert_eq!(bus.read32(RAM + 0x4014), 8);
    assert_eq!(cpu.regs[2], RAM + 0x4010);
}

#[test]
fn ldm_stm_wide_and_push_pop_w() {
    // e8a1 010c  stmia.w r1!, {r2, r3, r8}
    let (mut cpu, mut bus) = setup_w(&[(0xe8a1, 0x010c)]);
    cpu.regs[1] = RAM + 0x5000;
    cpu.regs[2] = 21;
    cpu.regs[3] = 22;
    cpu.regs[8] = 28;
    cpu.step(&mut bus);
    assert_eq!(bus.read32(RAM + 0x5000), 21);
    assert_eq!(bus.read32(RAM + 0x5004), 22);
    assert_eq!(bus.read32(RAM + 0x5008), 28);
    assert_eq!(cpu.regs[1], RAM + 0x500c);

    // e92d 4030  stmdb sp!, {r4, r5, lr}  (push.w)
    // e8bd 8030  ldmia.w sp!, {r4, r5, pc} (pop.w)
    let (mut cpu, mut bus) = setup_w(&[(0xe92d, 0x4030), (0xe8bd, 0x8030)]);
    let sp0 = cpu.regs[SP];
    cpu.regs[4] = 44;
    cpu.regs[5] = 55;
    cpu.regs[LR] = (RAM + 0x600) | 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[SP], sp0 - 12);
    assert_eq!(bus.read32(sp0 - 12), 44);
    assert_eq!(bus.read32(sp0 - 4), (RAM + 0x600) | 1);
    cpu.regs[4] = 0;
    cpu.regs[5] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[SP], sp0);
    assert_eq!((cpu.regs[4], cpu.regs[5]), (44, 55));
    assert_eq!(cpu.regs[PC], RAM + 0x600);
}

#[test]
fn tbb_tbh_dispatch() {
    // e8d0 f001  tbb [r0, r1]
    let (mut cpu, mut bus) = setup_w(&[(0xe8d0, 0xf001)]);
    cpu.regs[0] = RAM + 0x100; // table base
    cpu.regs[1] = 2; // index
    bus.write8(RAM + 0x102, 0x10); // entry: offset/2
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 4 + 0x20);

    // e8d0 f011  tbh [r0, r1, lsl #1]
    let (mut cpu, mut bus) = setup_w(&[(0xe8d0, 0xf011)]);
    cpu.regs[0] = RAM + 0x100;
    cpu.regs[1] = 3;
    bus.write16(RAM + 0x106, 0x200);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 4 + 0x400);
}

// --- Shifts (register), misc, multiply -------------------------------------------

#[test]
fn wide_register_shifts() {
    // fa01 f002  lsl.w r0, r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xfa01, 0xf002)]);
    cpu.regs[1] = 1;
    cpu.regs[2] = 31;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x8000_0000);

    // fa31 f002  lsrs.w r0, r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xfa31, 0xf002)]);
    cpu.regs[1] = 0x3;
    cpu.regs[2] = 1;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 1);
    assert!(cpu.c);
}

#[test]
fn clz_rbit_rev_wide() {
    // fab1 f081 clz ; fa91 f0a1 rbit ; fa91 f081 rev.w
    let (mut cpu, mut bus) = setup_w(&[(0xfab1, 0xf081), (0xfa91, 0xf0a1), (0xfa91, 0xf081)]);
    cpu.regs[1] = 0x0001_0000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 15);
    cpu.regs[1] = 0x8000_0001;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x8000_0001);
    cpu.regs[1] = 0x1122_3344;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x4433_2211);
}

#[test]
fn wide_extends() {
    // fa4f f081 sxtb.w ; fa5f f081 uxtb.w ; fa0f f081 sxth.w ; fa1f f081 uxth.w
    let (mut cpu, mut bus) = setup_w(&[
        (0xfa4f, 0xf081),
        (0xfa5f, 0xf081),
        (0xfa0f, 0xf081),
        (0xfa1f, 0xf081),
    ]);
    cpu.regs[1] = 0x0001_8291;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_ff91);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x91);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0xffff_8291);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x8291);
}

#[test]
fn multiply_family() {
    // fb01 f002 mul ; fb01 3002 mla ; fb01 3012 mls
    let (mut cpu, mut bus) = setup_w(&[(0xfb01, 0xf002), (0xfb01, 0x3002), (0xfb01, 0x3012)]);
    cpu.regs[1] = 6;
    cpu.regs[2] = 7;
    cpu.regs[3] = 100;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 42);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 142);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 58);
}

#[test]
fn long_multiply_and_divide() {
    // fba2 0103  umull r0, r1, r2, r3
    let (mut cpu, mut bus) = setup_w(&[(0xfba2, 0x0103)]);
    cpu.regs[2] = 0xffff_ffff;
    cpu.regs[3] = 2;
    cpu.step(&mut bus);
    assert_eq!((cpu.regs[0], cpu.regs[1]), (0xffff_fffe, 1));

    // fb82 0103  smull r0, r1, r2, r3
    let (mut cpu, mut bus) = setup_w(&[(0xfb82, 0x0103)]);
    cpu.regs[2] = (-3i32) as u32;
    cpu.regs[3] = 4;
    cpu.step(&mut bus);
    assert_eq!(
        (cpu.regs[0], cpu.regs[1]),
        ((-12i64) as u64 as u32, ((-12i64) >> 32) as u32)
    );

    // fbb1 f0f2 udiv ; fb91 f0f2 sdiv
    let (mut cpu, mut bus) = setup_w(&[(0xfbb1, 0xf0f2), (0xfb91, 0xf0f2)]);
    cpu.regs[1] = 100;
    cpu.regs[2] = 7;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 14);
    cpu.regs[1] = (-100i32) as u32;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], (-14i32) as u32);

    // Division by zero yields 0 (DIV_0_TRP clear).
    let (mut cpu, mut bus) = setup_w(&[(0xfbb1, 0xf0f2)]);
    cpu.regs[1] = 5;
    cpu.regs[2] = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
}

// --- Wide branches ------------------------------------------------------------

#[test]
fn cond_branch_wide() {
    // f000 807e  beq.w .+0x100 (offset from pc4 = 0xfc... target pc+0x100)
    let (mut cpu, mut bus) = setup_w(&[(0xf000, 0x807e)]);
    cpu.z = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x100);

    let (mut cpu, mut bus) = setup_w(&[(0xf000, 0x807e)]);
    cpu.z = false;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 4);

    // f47f af7e  bne.w .-0x100
    let (mut cpu, mut bus) = setup_w(&[]);
    bus.write16(RAM + 0x200, 0xf47f);
    bus.write16(RAM + 0x202, 0xaf7e);
    cpu.regs[PC] = RAM + 0x200;
    cpu.z = false;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x100);
}

#[test]
fn uncond_branch_wide() {
    // f000 bffe  b.w .+0x1000
    let (mut cpu, mut bus) = setup_w(&[(0xf000, 0xbffe)]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x1000);

    // f7fe bffe  b.w .-0x1000
    let (mut cpu, mut bus) = setup_w(&[]);
    bus.write16(RAM + 0x2000, 0xf7fe);
    bus.write16(RAM + 0x2002, 0xbffe);
    cpu.regs[PC] = RAM + 0x2000;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], RAM + 0x1000);
}

#[test]
fn exclusive_pair() {
    // e851 0f00  ldrex r0, [r1] ; e841 2000  strex r0, r2, [r1]
    let (mut cpu, mut bus) = setup_w(&[(0xe851, 0x0f00), (0xe841, 0x2000)]);
    cpu.regs[1] = RAM + 0x6000;
    cpu.regs[2] = 0x1234;
    bus.write32(RAM + 0x6000, 0x4321);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0x4321);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0); // success
    assert_eq!(bus.read32(RAM + 0x6000), 0x1234);
}

#[test]
fn halfword_and_high_multiplies() {
    // fb19 4608  smlabb r6, r9, r8, r4
    let (mut cpu, mut bus) = setup_w(&[(0xfb19, 0x4608)]);
    cpu.regs[9] = 0xaaaa_0007; // bottom = 7
    cpu.regs[8] = 0xbbbb_0003; // bottom = 3
    cpu.regs[4] = 100;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[6], 121);

    // fb11 3022  smlatb r0, r1, r2, r3 — top(r1) * bottom(r2) + r3.
    let (mut cpu, mut bus) = setup_w(&[(0xfb11, 0x3022)]);
    cpu.regs[1] = 0xfffe_0000; // top = -2
    cpu.regs[2] = 0x0000_0005; // bottom = 5
    cpu.regs[3] = 3;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], (-7i32) as u32);

    // fb11 f032  smultt r0, r1, r2 — tops, no accumulate.
    let (mut cpu, mut bus) = setup_w(&[(0xfb11, 0xf032)]);
    cpu.regs[1] = 0x0004_ffff;
    cpu.regs[2] = 0x0009_ffff;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 36);

    // fb31 f002  smulwb r0, r1, r2 — (r1 * bottom(r2)) >> 16.
    let (mut cpu, mut bus) = setup_w(&[(0xfb31, 0xf002)]);
    cpu.regs[1] = 0x0001_0000; // 65536
    cpu.regs[2] = 0x0000_0003;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 3);

    // fb51 f002  smmul r0, r1, r2 — high word of signed product.
    let (mut cpu, mut bus) = setup_w(&[(0xfb51, 0xf002)]);
    cpu.regs[1] = 0x4000_0000;
    cpu.regs[2] = 8;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 2);

    // fb51 3002  smmla r0, r1, r2, r3.
    let (mut cpu, mut bus) = setup_w(&[(0xfb51, 0x3002)]);
    cpu.regs[1] = 0x4000_0000;
    cpu.regs[2] = 8;
    cpu.regs[3] = 5;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 7);
}

#[test]
fn extend_and_add_family() {
    // fa17 f585  uxtah r5, r7, r5 — the encoding that broke tinyusb's
    // cdcd_open when misdecoded as a register shift.
    let (mut cpu, mut bus) = setup_w(&[(0xfa17, 0xf585)]);
    cpu.regs[7] = 9;
    cpu.regs[5] = 0xffff_0030; // halfword 0x30 = 48
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[5], 9 + 48);

    // fa01 f082  sxtah r0, r1, r2 — signed halfword accumulate.
    let (mut cpu, mut bus) = setup_w(&[(0xfa01, 0xf082)]);
    cpu.regs[1] = 100;
    cpu.regs[2] = 0x0000_fffe; // -2 as i16
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 98);

    // fa51 f082  uxtab r0, r1, r2.
    let (mut cpu, mut bus) = setup_w(&[(0xfa51, 0xf082)]);
    cpu.regs[1] = 1000;
    cpu.regs[2] = 0x1_02ff;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 1000 + 0xff);

    // fa41 f082  sxtab r0, r1, r2.
    let (mut cpu, mut bus) = setup_w(&[(0xfa41, 0xf082)]);
    cpu.regs[1] = 50;
    cpu.regs[2] = 0x80; // -128 as i8
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 50u32.wrapping_sub(128));

    // fa11 f092  uxtah r0, r1, r2, ror #8.
    let (mut cpu, mut bus) = setup_w(&[(0xfa11, 0xf092)]);
    cpu.regs[1] = 5;
    cpu.regs[2] = 0x0012_3400; // ror 8 -> 0x0000_1234 -> halfword 0x1234
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 5 + 0x1234);
}

#[test]
fn fpu_basic_arithmetic_and_moves() {
    // ee01 2a90 vmov s3, r2 ; ee30 1a81 vadd.f32... build a sequence:
    // vmov s1, r1 ; vmov s2, r2 ; vadd.f32 s0, s1, s2 ; vmov r0, s0
    let (mut cpu, mut bus) = setup_w(&[
        (0xee00, 0x1a90), // vmov s1, r1
        (0xee01, 0x2a10), // vmov r2... careful: use encodings from as below
    ]);
    // Simpler: drive exec_fp through real encodings one at a time.
    cpu.regs[1] = 1.5f32.to_bits();
    cpu.regs[2] = 2.5f32.to_bits();
    // vmov s1, r1 = ee00 1a90 ; vmov s2, r2 = ee01 2a10
    bus.write16(RAM, 0xee00);
    bus.write16(RAM + 2, 0x1a90);
    bus.write16(RAM + 4, 0xee01);
    bus.write16(RAM + 6, 0x2a10);
    // vadd.f32 s0, s1, s2 = ee30 0a81
    bus.write16(RAM + 8, 0xee30);
    bus.write16(RAM + 10, 0x0a81);
    // vmul.f32 s3, s1, s2 = ee20 1a81 -> s3? vmul.f32 s3,s1,s2: d=3
    bus.write16(RAM + 12, 0xee60);
    bus.write16(RAM + 14, 0x1a81); // vmul.f32 s3, s1, s2 (D=1 -> eeXX with bit6)
    // vmov r0, s0 = ee10 0a10
    bus.write16(RAM + 16, 0xee10);
    bus.write16(RAM + 18, 0x0a10);
    // vmov r3, s3 = ee11 3a90
    bus.write16(RAM + 20, 0xee11);
    bus.write16(RAM + 22, 0x3a90);
    for _ in 0..6 {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.break_cause, None);
    assert_eq!(f32::from_bits(cpu.regs[0]), 4.0);
    assert_eq!(f32::from_bits(cpu.regs[3]), 3.75);
}

#[test]
fn fpu_div_sqrt_cvt_cmp() {
    let (mut cpu, mut bus) = setup_w(&[]);
    cpu.fpregs[1] = 7.0f32.to_bits();
    cpu.fpregs[2] = 2.0f32.to_bits();
    let seq: &[(u16, u16)] = &[
        (0xee80, 0x0a81), // vdiv.f32 s0, s1, s2 = 3.5
        (0xeeb1, 0x2ac1), // vsqrt.f32 s4, s2    = sqrt(2)
        (0xeebd, 0x3a40), // vcvt.s32.f32 s6, s0 = 3
        (0xeeb4, 0x0a41), // vcmp.f32 s0, s2     (3.5 > 2.0 -> C, !N, !Z)
        (0xeef1, 0xfa10), // vmrs APSR_nzcv, fpscr
    ];
    for (i, (a, b)) in seq.iter().enumerate() {
        bus.write16(RAM + 4 * i as u32, *a);
        bus.write16(RAM + 4 * i as u32 + 2, *b);
    }
    for _ in 0..seq.len() {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.break_cause, None);
    assert_eq!(f32::from_bits(cpu.fpregs[0]), 3.5);
    assert!((f32::from_bits(cpu.fpregs[4]) - 2.0f32.sqrt()).abs() < 1e-6);
    assert_eq!(cpu.fpregs[6] as i32, 3);
    assert!(cpu.c && !cpu.n && !cpu.z);
}

#[test]
fn fpu_vldr_vstr_vpush_vpop_and_imm() {
    let (mut cpu, mut bus) = setup_w(&[]);
    let sp0 = cpu.regs[SP];
    bus.write32(RAM + 0x1000, 6.25f32.to_bits());
    cpu.regs[1] = RAM + 0x1000;
    cpu.fpregs[16] = 1.0f32.to_bits();
    cpu.fpregs[17] = 2.0f32.to_bits();
    cpu.fpregs[18] = 3.0f32.to_bits();
    let seq: &[(u16, u16)] = &[
        (0xed91, 0x0a00), // vldr s0, [r1]
        (0xed81, 0x0a04), // vstr s0, [r1, #16]
        (0xed2d, 0x8a03), // vpush {s16-s18}
        (0xeeb7, 0x0a00), // vmov.f32 s0, #1.0
        (0xeef8, 0x0a04), // vmov.f32 s1, #-2.5
        (0xecbd, 0x8a03), // vpop {s16-s18}
    ];
    for (i, (a, b)) in seq.iter().enumerate() {
        bus.write16(RAM + 4 * i as u32, *a);
        bus.write16(RAM + 4 * i as u32 + 2, *b);
    }
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(cpu.fpregs[0]), 6.25);
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(bus.read32(RAM + 0x1010)), 6.25);
    cpu.step(&mut bus); // vpush
    assert_eq!(cpu.regs[SP], sp0 - 12);
    assert_eq!(f32::from_bits(bus.read32(sp0 - 12)), 1.0);
    cpu.fpregs[16] = 0;
    cpu.fpregs[17] = 0;
    cpu.fpregs[18] = 0;
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(cpu.fpregs[0]), 1.0); // vmov imm
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(cpu.fpregs[1]), -2.5);
    cpu.step(&mut bus); // vpop
    assert_eq!(cpu.regs[SP], sp0);
    assert_eq!(f32::from_bits(cpu.fpregs[17]), 2.0);
    assert_eq!(cpu.break_cause, None);
}

// --- SSAT / USAT (plain binary immediate) ------------------------------------

#[test]
fn usat_clamps_and_sets_q() {
    // f380 0008  usat r0, #8, r0   (the sl_device_init_hfxo CTUNE op)
    let (mut cpu, mut bus) = setup_w(&[(0xf380, 0x0008)]);
    cpu.regs[0] = 0x1234; // 4660 > 255
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 255);
    assert!(cpu.q);

    // In-range value: unchanged, Q stays sticky from before but starts
    // clean on a fresh core.
    let (mut cpu, mut bus) = setup_w(&[(0xf380, 0x0008)]);
    cpu.regs[0] = 200;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 200);
    assert!(!cpu.q);

    // Negative input clamps to 0.
    let (mut cpu, mut bus) = setup_w(&[(0xf380, 0x0008)]);
    cpu.regs[0] = (-5i32) as u32;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 0);
    assert!(cpu.q);
}

#[test]
fn usat_with_lsl_shift() {
    // f381 1004  usat r0, #4, r1, lsl #4  (imm3=001, imm2=00 -> amt 4)
    let (mut cpu, mut bus) = setup_w(&[(0xf381, 0x1004)]);
    cpu.regs[1] = 0x3; // 3 << 4 = 48 > 15
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 15);
    assert!(cpu.q);
}

#[test]
fn ssat_clamps_signed_both_ways() {
    // f301 0007  ssat r0, #8, r1   (n=8: range -128..=127)
    let (mut cpu, mut bus) = setup_w(&[(0xf301, 0x0007)]);
    cpu.regs[1] = 300;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0], 127);
    assert!(cpu.q);

    let (mut cpu, mut bus) = setup_w(&[(0xf301, 0x0007)]);
    cpu.regs[1] = (-300i32) as u32;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0] as i32, -128);
    assert!(cpu.q);

    // In range, incl. negative: value preserved, Q clear.
    let (mut cpu, mut bus) = setup_w(&[(0xf301, 0x0007)]);
    cpu.regs[1] = (-100i32) as u32;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0] as i32, -100);
    assert!(!cpu.q);
}

#[test]
fn ssat_with_asr_shift() {
    // f321 0087  ssat r0, #8, r1, asr #2  (sh=1, imm3=000 imm2=10 -> amt 2)
    let (mut cpu, mut bus) = setup_w(&[(0xf321, 0x0087)]);
    cpu.regs[1] = (-1024i32) as u32; // -1024 >> 2 = -256 -> clamps to -128
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[0] as i32, -128);
    assert!(cpu.q);
}

// --- VCVT fixed-point <-> F32 -------------------------------------------------

#[test]
fn vcvt_fixed_q15_to_float_and_back() {
    // eeba 6ae8  vcvt.f32.s32 s12, s12, #15  (the IADC_init Q15 op)
    let (mut cpu, mut bus) = setup_w(&[(0xeeba, 0x6ae8)]);
    cpu.cpacr = 0x00f0_0000; // CP10/11 full access
    cpu.fpregs[12] = 0x0000_8000; // 1.0 in Q15
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(cpu.fpregs[12]), 1.0);

    // Negative Q15: -0.5 = 0xFFFFC000 as i32.
    let (mut cpu, mut bus) = setup_w(&[(0xeeba, 0x6ae8)]);
    cpu.cpacr = 0x00f0_0000;
    cpu.fpregs[12] = (-16384i32) as u32;
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(cpu.fpregs[12]), -0.5);

    // eebe 6ae8  vcvt.s32.f32 s12, s12, #15  (float -> Q15, RZ)
    let (mut cpu, mut bus) = setup_w(&[(0xeebe, 0x6ae8)]);
    cpu.cpacr = 0x00f0_0000;
    cpu.fpregs[12] = 0.75f32.to_bits();
    cpu.step(&mut bus);
    assert_eq!(cpu.fpregs[12], 24576); // 0.75 * 2^15
}

#[test]
fn vcvt_fixed_unsigned_and_16bit_container() {
    // eebb 6ae8  vcvt.f32.u32 s12, s12, #15
    let (mut cpu, mut bus) = setup_w(&[(0xeebb, 0x6ae8)]);
    cpu.cpacr = 0x00f0_0000;
    cpu.fpregs[12] = 0x0001_8000; // 3.0 in unsigned Q15
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(cpu.fpregs[12]), 3.0);

    // eeba 6a67  vcvt.f32.s16 s12, s12, #16 (sx=0: 16-bit, imm5=0 -> fbits 16)
    // h2 = 0110 1010 0110 0111: sx bit7=0, i bit5=1, imm4=7 -> imm5=15, fbits=1
    let (mut cpu, mut bus) = setup_w(&[(0xeeba, 0x6a67)]);
    cpu.cpacr = 0x00f0_0000;
    cpu.fpregs[12] = 0x0000_0003; // 3 in S16 Q1 = 1.5
    cpu.step(&mut bus);
    assert_eq!(f32::from_bits(cpu.fpregs[12]), 1.5);
}

// --- ARMv7E-M DSP parallel add/subtract + SEL (GNU as, cortex-m7) ------------

#[test]
fn uadd8_sets_ge_and_sel_selects_bytes() {
    // fa81 f042  uadd8 r0, r1, r2 ; faa4 f385  sel r3, r4, r5
    let (mut cpu, mut bus) = setup_w(&[(0xfa81, 0xf042), (0xfaa4, 0xf385)]);
    cpu.regs[1] = 0x01ff_01ff; // bytes 3..0: 01 FF 01 FF
    cpu.regs[2] = 0x0101_ffff; //           01 01 FF FF
    cpu.step(&mut bus);
    // byte0 FF+FF=1FE, byte1 01+FF=100, byte2 FF+01=100 (all carry, GE=1);
    // byte3 01+01=02 (no carry, GE=0).
    assert_eq!(cpu.regs[0], 0x0200_00fe);
    assert_eq!(cpu.ge, 0b0111);

    // SEL picks Rn byte where GE=1, else Rm byte.
    cpu.regs[4] = 0xaaaa_aaaa;
    cpu.regs[5] = 0x5555_5555;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[3], 0x55aa_aaaa, "GE 0111 -> bytes 0-2 from Rn");
}

#[test]
fn usub8_ge_is_no_borrow() {
    // fac1 f042  usub8 r0, r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xfac1, 0xf042)]);
    cpu.regs[1] = 0x1020_0580; // bytes 3..0: 10 20 05 80
    cpu.regs[2] = 0x0520_1001; //           05 20 10 01
    cpu.step(&mut bus);
    // byte0 80-01=7F (GE), byte1 05-10 borrow (no GE), byte2 20-20=00 (GE),
    // byte3 10-05=0B (GE).
    assert_eq!(cpu.regs[0], 0x0b00_f57f);
    assert_eq!(cpu.ge, 0b1101);
}

#[test]
fn uadd16_two_lanes_and_ge_pairs() {
    // fa91 f042  uadd16 r0, r1, r2
    let (mut cpu, mut bus) = setup_w(&[(0xfa91, 0xf042)]);
    cpu.regs[1] = 0xffff_0001; // hi 0xffff, lo 0x0001
    cpu.regs[2] = 0x0002_0002; // hi 0x0002, lo 0x0002
    cpu.step(&mut bus);
    // lo 0x0001+0x0002=0x0003 (no carry, GE[1:0]=0);
    // hi 0xffff+0x0002=0x10001 -> 0x0001 (carry, GE[3:2]=1).
    assert_eq!(cpu.regs[0], 0x0001_0003);
    assert_eq!(cpu.ge, 0b1100);
}

#[test]
fn apsr_ge_round_trips_through_xpsr() {
    let mut cpu = CortexM7::new();
    cpu.ge = 0b1010;
    assert_eq!((cpu.xpsr() >> 16) & 0xf, 0b1010);
    cpu.set_apsr(0x0005_0000); // GE = 0b0101
    assert_eq!(cpu.ge, 0b0101);
}

// --- FPv5-D16 double-precision (CLOCK_GetPllFreq's f64 (a*b)/c path) ---------

#[test]
fn double_precision_mul_div_and_int_convert() {
    // The exact sequence the RT1062 SDK compiles for a PLL frequency ratio.
    let (mut cpu, mut bus) = setup_w(&[
        (0xeeb8, 0x7b67), // vcvt.f64.u32 d7, s15   ; D7 = (f64) S15
        (0xee27, 0x7b06), // vmul.f64 d7, d7, d6    ; D7 = D7 * D6
        (0xee87, 0x6b05), // vdiv.f64 d6, d7, d5    ; D6 = D7 / D5
        (0xeefc, 0x7bc6), // vcvt.u32.f64 s15, d6   ; S15 = (u32) D6
    ]);
    cpu.cpacr = 0x00f0_0000; // CP10/11 full access
    cpu.fpregs[15] = 6; // S15 = 6 (u32)
    let set_d = |cpu: &mut CortexM7, d: usize, v: f64| {
        let b = v.to_bits();
        cpu.fpregs[d * 2] = b as u32;
        cpu.fpregs[d * 2 + 1] = (b >> 32) as u32;
    };
    set_d(&mut cpu, 6, 7.0); // D6 = 7.0
    set_d(&mut cpu, 5, 3.0); // D5 = 3.0
    for _ in 0..4 {
        cpu.step(&mut bus);
    }
    // (6 * 7) / 3 = 14.
    assert_eq!(cpu.fpregs[15], 14);
    assert!(cpu.break_cause.is_none());
}

#[test]
fn double_precision_add_aliases_s_pairs() {
    // ee31 0b02  vadd.f64 d0, d1, d2
    let (mut cpu, mut bus) = setup_w(&[(0xee31, 0x0b02)]);
    cpu.cpacr = 0x00f0_0000;
    let set_d = |cpu: &mut CortexM7, d: usize, v: f64| {
        let b = v.to_bits();
        cpu.fpregs[d * 2] = b as u32;
        cpu.fpregs[d * 2 + 1] = (b >> 32) as u32;
    };
    set_d(&mut cpu, 1, 1.5); // D1
    set_d(&mut cpu, 2, 2.75); // D2
    cpu.step(&mut bus);
    let d0 = f64::from_bits(cpu.fpregs[0] as u64 | (cpu.fpregs[1] as u64) << 32);
    assert_eq!(d0, 4.25);
}

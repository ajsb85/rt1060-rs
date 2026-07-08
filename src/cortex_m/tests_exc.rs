//! Exception, NVIC, and SysTick tests.

use super::asm::*;
use super::tests::TestBus;
use super::*;

const RAM: u32 = 0x2000_0000;
const VTOR: u32 = RAM + 0x8000;
const HANDLER: u32 = RAM + 0x9000;

/// Core with a vector table at VTOR: entry `n` points at HANDLER + 0x40*n.
fn setup_exc(code: &[u16]) -> (CortexM7, TestBus) {
    let mut cpu = CortexM7::new();
    let mut bus = TestBus {
        ram: vec![0; 0x4_0000],
    };
    cpu.regs[PC] = RAM;
    cpu.regs[SP] = RAM + 0x2_0000;
    cpu.vtor = VTOR;
    for n in 0..(16 + NUM_IRQS as u32) {
        bus.write32(VTOR + 4 * n, (HANDLER + 0x40 * n) | 1);
    }
    for (i, hw) in code.iter().enumerate() {
        bus.write16(RAM + 2 * i as u32, *hw);
    }
    (cpu, bus)
}

fn handler_addr(n: u32) -> u32 {
    HANDLER + 0x40 * n
}

#[test]
fn svc_enters_handler_and_bx_lr_returns() {
    let (mut cpu, mut bus) = setup_exc(&[svc(7), nop()]);
    let sp0 = cpu.regs[SP];
    cpu.n = true; // must survive the round trip
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[PC], handler_addr(11));
    assert_eq!(cpu.ipsr, 11);
    assert_eq!(cpu.regs[SP], sp0 - 0x20); // frame pushed
    assert_eq!(cpu.regs[LR], 0xffff_fff9); // thread, MSP, no FP
    // Frame: return address points after the SVC, xPSR has T set.
    assert_eq!(bus.read32(sp0 - 8), RAM + 2);
    assert!(bus.read32(sp0 - 4) & 1 << 24 != 0);

    // Handler: BX LR returns.
    bus.write16(handler_addr(11), 0x4770); // bx lr
    cpu.step(&mut bus); // executes BX LR, queues the return
    cpu.step(&mut bus); // performs return + executes NOP at RAM+2
    assert_eq!(cpu.ipsr, 0);
    assert_eq!(cpu.regs[SP], sp0);
    assert!(cpu.n, "APSR must be restored from the stacked xPSR");
    assert_eq!(cpu.regs[PC], RAM + 4); // NOP retired
}

#[test]
fn nvic_irq_fires_when_enabled_and_unmasked() {
    let (mut cpu, mut bus) = setup_exc(&[nop(), nop(), nop()]);
    // Pend IRQ 3 via ISPR, but leave it disabled: nothing happens.
    cpu.step(&mut bus);
    cpu.nvic_pending.set(3);
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 0);
    // Enable it: taken before the next instruction.
    cpu.nvic_enable.set(3);
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 16 + 3);
    assert_eq!(cpu.regs[PC], handler_addr(16 + 3) + 2); // handler ran 1 insn
    assert!(!cpu.nvic_pending.test(3)); // pending cleared on entry
}

#[test]
fn primask_defers_irq_until_cpsie() {
    let (mut cpu, mut bus) = setup_exc(&[cpsid_i(), nop(), cpsie_i(), nop()]);
    cpu.step(&mut bus); // CPSID i
    cpu.nvic_pending.set(0);
    cpu.nvic_enable.set(0);
    cpu.step(&mut bus); // NOP: IRQ masked
    assert_eq!(cpu.ipsr, 0);
    cpu.step(&mut bus); // CPSIE i
    cpu.step(&mut bus); // IRQ taken before the last NOP
    assert_eq!(cpu.ipsr, 16);
}

#[test]
fn higher_priority_preempts_lower() {
    let (mut cpu, mut bus) = setup_exc(&[nop(), nop()]);
    cpu.nvic_enable.set(0);
    cpu.nvic_enable.set(1);
    cpu.nvic_prio[0] = 0x80;
    cpu.nvic_prio[1] = 0x40; // IRQ1 outranks IRQ0
    cpu.nvic_pending.set(0);
    cpu.step(&mut bus); // take IRQ0, run first handler insn
    assert_eq!(cpu.ipsr, 16);
    // Now pend the higher-priority IRQ1: preempts the running handler.
    cpu.nvic_pending.set(1);
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 17);
    assert_eq!(cpu.regs[LR], 0xffff_fff1); // came from handler mode

    // An equal/lower priority interrupt would NOT preempt.
    let (mut cpu, mut bus) = setup_exc(&[nop(), nop()]);
    cpu.nvic_enable.set(0);
    cpu.nvic_enable.set(1);
    cpu.nvic_pending.set(0);
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 16);
    cpu.nvic_pending.set(1); // same priority (0)
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 16, "equal priority must not preempt");
}

#[test]
fn nested_return_restores_outer_handler() {
    let (mut cpu, mut bus) = setup_exc(&[nop(), nop(), nop(), nop()]);
    cpu.nvic_enable.set(0);
    cpu.nvic_enable.set(1);
    cpu.nvic_prio[0] = 0x80;
    cpu.nvic_prio[1] = 0x40;
    cpu.nvic_pending.set(0);
    cpu.step(&mut bus); // in IRQ0 handler
    cpu.nvic_pending.set(1);
    cpu.step(&mut bus); // preempted into IRQ1 handler
    assert_eq!(cpu.ipsr, 17);
    bus.write16(cpu.regs[PC], 0x4770); // bx lr at current position
    cpu.step(&mut bus); // BX LR
    cpu.step(&mut bus); // exception return performed
    assert_eq!(cpu.ipsr, 16, "must return to the outer IRQ0 handler");
}

#[test]
fn systick_counts_down_and_fires() {
    let (mut cpu, mut bus) = setup_exc(&[nop(); 32]);
    cpu.syst_rvr = 5;
    cpu.syst_cvr = 5;
    cpu.syst_csr = 0b111; // ENABLE | TICKINT | CLKSOURCE (processor clock)
    let mut fired_at = None;
    for i in 0..12 {
        cpu.step(&mut bus);
        if cpu.ipsr == 15 {
            fired_at = Some(i);
            break;
        }
    }
    let fired_at = fired_at.expect("SysTick handler must be entered");
    assert!(
        fired_at >= 4,
        "must count RVR ticks first (fired at {fired_at})"
    );
    assert_eq!(cpu.regs[PC], handler_addr(15) + 2);
    // COUNTFLAG visible via CSR read (and cleared by it).
    // (handler entry consumed nothing from CSR)
    assert!(cpu.syst_csr & 1 << 16 != 0);
}

#[test]
fn pendsv_via_icsr_write() {
    let (mut cpu, mut bus) = setup_exc(&[nop(), nop()]);
    // MSR-less path: write ICSR through a store.
    cpu.ppb_write(0xe000_ed04, 1 << 28);
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 14);
    assert!(!cpu.icsr_pendsv);
}

#[test]
fn exception_frame_alignment_padding() {
    let (mut cpu, mut bus) = setup_exc(&[svc(0)]);
    cpu.regs[SP] = RAM + 0x2_0000 - 4; // 4-byte aligned, not 8
    let sp0 = cpu.regs[SP];
    cpu.step(&mut bus);
    // Frame is 8-byte aligned; pad flag lives in stacked xPSR bit 9.
    assert_eq!(cpu.regs[SP], (sp0 - 0x20) & !0x7);
    let stacked_xpsr = bus.read32(cpu.regs[SP] + 28);
    assert!(stacked_xpsr & 1 << 9 != 0);
    // Return restores the padded SP exactly.
    bus.write16(handler_addr(11), 0x4770);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs[SP], sp0);
}

#[test]
fn irq_lines_from_bus_pend_interrupts() {
    struct IrqBus {
        inner: TestBus,
        line: IrqMask,
    }
    impl Bus for IrqBus {
        fn read8(&mut self, a: u32) -> u8 {
            self.inner.read8(a)
        }
        fn read16(&mut self, a: u32) -> u16 {
            self.inner.read16(a)
        }
        fn read32(&mut self, a: u32) -> u32 {
            self.inner.read32(a)
        }
        fn write8(&mut self, a: u32, v: u8) {
            self.inner.write8(a, v)
        }
        fn write16(&mut self, a: u32, v: u16) {
            self.inner.write16(a, v)
        }
        fn write32(&mut self, a: u32, v: u32) {
            self.inner.write32(a, v)
        }
        fn irq_lines(&mut self) -> IrqMask {
            self.line
        }
    }
    let (mut cpu, bus) = setup_exc(&[nop(), nop()]);
    let mut bus = IrqBus {
        inner: bus,
        line: IrqMask::ZERO,
    };
    cpu.nvic_enable.set(20); // LPUART1_IRQn on the i.MX RT1060
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 0);
    let mut line = IrqMask::ZERO;
    line.set(20);
    bus.line = line;
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 16 + 20);
}

#[test]
fn systick_registers_via_ppb() {
    let mut cpu = CortexM7::new();
    cpu.ppb_write(0xe000_e014, 0x1234);
    assert_eq!(cpu.ppb_read(0xe000_e014), 0x1234);
    cpu.syst_csr = 1 << 16 | 1;
    assert_eq!(cpu.ppb_read(0xe000_e010), 0x1_0001);
    // COUNTFLAG cleared by the read.
    assert_eq!(cpu.ppb_read(0xe000_e010), 0x1);
    // CVR write clears the counter.
    cpu.syst_cvr = 55;
    cpu.ppb_write(0xe000_e018, 0xdead);
    assert_eq!(cpu.syst_cvr, 0);
}

#[test]
fn nvic_ipr_byte_granularity() {
    let mut cpu = CortexM7::new();
    // Word write to IPR0 sets IRQs 0-3 (top nibble kept).
    cpu.ppb_write(0xe000_e400, 0x44_33_22_11);
    assert_eq!(cpu.nvic_prio[0], 0x10);
    assert_eq!(cpu.nvic_prio[1], 0x20);
    assert_eq!(cpu.nvic_prio[2], 0x30);
    assert_eq!(cpu.nvic_prio[3], 0x40);
    assert_eq!(cpu.ppb_read(0xe000_e400), 0x40_30_20_10);
    // IPR39 is the last word: IRQ 156 (lane 0) and IRQ 157 (lane 1,
    // GPIO6_7_8_9). Lanes 2-3 map to IRQs 158-159, which are RAZ/WI.
    cpu.ppb_write(0xe000_e49c, 0xff_ff_f0_f0);
    assert_eq!(cpu.nvic_prio[156], 0xf0);
    assert_eq!(cpu.nvic_prio[157], 0xf0);
    assert_eq!(cpu.ppb_read(0xe000_e49c), 0x00_00_f0_f0);
}

#[test]
fn nvic_fifth_word_irq_above_127() {
    // 158 i.MX RT1060 IRQs need ISER/ISPR/ICER/ICPR words 0-4; IRQ 140
    // (PWM3_2) is bit 12 of the fifth word (0xe000_e110) — this pins the
    // widened [`IrqMask`] plumbing that a u128 could not carry.
    let (mut cpu, mut bus) = setup_exc(&[nop(), nop()]);
    cpu.ppb_write(0xe000_e110, 1 << 12); // ISER4: enable IRQ 140
    cpu.ppb_write(0xe000_e210, 1 << 12); // ISPR4: pend IRQ 140
    assert_eq!(cpu.ppb_read(0xe000_e110), 1 << 12);
    assert_eq!(cpu.ppb_read(0xe000_e190), 1 << 12); // ICER4 reads like ISER4
    assert_eq!(cpu.ppb_read(0xe000_e210), 1 << 12);
    cpu.step(&mut bus);
    assert_eq!(cpu.ipsr, 16 + 140);
    assert_eq!(cpu.regs[PC], handler_addr(16 + 140) + 2);
    assert_eq!(cpu.ppb_read(0xe000_e210), 0); // pending cleared on entry
    // ICER4/ICPR4 clear their bits; bits above IRQ 157 are RAZ/WI.
    cpu.ppb_write(0xe000_e190, 1 << 12);
    assert_eq!(cpu.ppb_read(0xe000_e110), 0);
    cpu.ppb_write(0xe000_e110, 0xffff_ffff); // only bits 0..=29 (IRQ 128-157) exist
    assert_eq!(cpu.ppb_read(0xe000_e110), 0x3fff_ffff);
}

#[test]
fn mpu_registers_stored_readback() {
    // The Zephyr/MCUXpresso startup programs MPU regions at boot and only
    // needs the writes to stick. No enforcement is modeled.
    let mut cpu = CortexM7::new();
    // MPU_TYPE: 16 data regions (Cortex-M7 PMSAv7), no instruction regions.
    assert_eq!(cpu.ppb_read(0xe000_ed90), 0x0000_1000);
    cpu.ppb_write(0xe000_ed94, 0x5); // MPU_CTRL: ENABLE | PRIVDEFENA
    assert_eq!(cpu.ppb_read(0xe000_ed94), 0x5);
    cpu.ppb_write(0xe000_ed98, 3); // MPU_RNR
    cpu.ppb_write(0xe000_ed9c, 0x2000_0000 | 0x1); // MPU_RBAR: base | XN
    cpu.ppb_write(0xe000_eda0, 0x2003_ffe0 | 0x1); // MPU_RLAR: limit | EN
    cpu.ppb_write(0xe000_edc0, 0x0000_0044); // MPU_MAIR0
    cpu.ppb_write(0xe000_edc4, 0x0000_00ee); // MPU_MAIR1
    assert_eq!(cpu.ppb_read(0xe000_ed98), 3);
    assert_eq!(cpu.ppb_read(0xe000_ed9c), 0x2000_0001);
    assert_eq!(cpu.ppb_read(0xe000_eda0), 0x2003_ffe1);
    assert_eq!(cpu.ppb_read(0xe000_edc0), 0x44);
    assert_eq!(cpu.ppb_read(0xe000_edc4), 0xee);
}

// (No SAU test: the Cortex-M7 is Armv7E-M and has no TrustZone/SAU. The
// vestigial SAU stored-readback registers in the core are never exercised
// by real i.MX RT1060 firmware.)

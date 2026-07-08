# DESIGN — rt1060-rs architecture (pinned)

This document pins the architecture. Read it before writing code. It mirrors
the rp2350-rs / mg24-rs design, adapted to the i.MX RT1060.

## §1 Execution model

Plain data, split borrows, no dynamic dispatch in the hot path:

```
CortexM7 (register state)  --step(&mut bus)-->  SystemBus (memory + peripherals)
```

- The core owns registers, flags, NVIC/SysTick/MPU state, and the FPU
  register file. It borrows a `Bus` for the duration of one `step`.
- `SystemBus` owns the RAM-like regions and a `Peripherals` aggregate.
- No `Rc<RefCell>`, no `Box<dyn Trait>` on the execution path; no per-step
  allocation; no hashing in `read*/write*`.

## §2 The CPU: Cortex-M7 vs the ported M33

The Cortex-M7 (Armv7E-M) and the EFR32's Cortex-M33 (Armv8-M Mainline) run
the **same Thumb-2 mainline + DSP instruction set**, so the decoder and
executor port unchanged from mg24-rs. Adaptations pinned here:

- **158 external interrupts** (`MIMXRT1062.h`, last = `GPIO6_7_8_9_IRQn`
  = 157). A `u128` cannot hold them; the NVIC masks are an `IrqMask` newtype
  of `[u32; 5]` (bit `32*w+b` = IRQ `32*w+b`, mapping 1:1 to the ISER/ICER/
  ISPR/ICPR banks by word index `addr[4:2]`).
- **CPUID = `0x411F_C272`** (Cortex-M7 r1p2).
- **MPU = PMSAv7, 16 regions** (`MPU_TYPE.DREGION = 16`), stored-readback,
  no enforcement (matches the mg24-rs precedent).
- **No TrustZone / SAU** — the SAU registers inherited from the M33 port are
  vestigial and never gate access; real M7 firmware never touches them.
- **FPU = FPv5-D16** (double precision). The single-precision path is ported;
  double-precision ops and 64-bit `vldm/vstm` widening are a ROADMAP item.
  SwiftIO firmware is built soft-float (`nofp`), so this does not block boot.
- **L1 I/D caches** are modeled as coherent no-ops; cache-maintenance `mcr`
  ops are accepted and ignored.
- The PPB (`0xE000_0000+`) is intercepted in-core and never reaches the bus.

## §3 Memory map (SwiftIO Micro)

| region         | base          | size    | backing            |
|----------------|---------------|---------|--------------------|
| ITCM           | `0x0000_0000` | 512 KiB | zeroed RAM         |
| Boot ROM       | `0x0020_0000` | —       | stub (ROADMAP)     |
| DTCM           | `0x2000_0000` | 512 KiB | zeroed RAM         |
| OCRAM          | `0x2020_0000` | 1 MiB   | zeroed RAM         |
| AIPS periph.   | `0x4000_0000` | 64 MiB  | `Peripherals`      |
| GPIO6–9 island | `0x4200_0000` | —       | `Peripherals`      |
| FlexSPI NOR    | `0x6000_0000` | 16 MiB  | `0xFF`-filled      |
| SEMC SDRAM     | `0x8000_0000` | 32 MiB  | zeroed RAM         |

The TCM/OCRAM split is FlexRAM-configurable via `IOMUXC_GPR`; we model the
maximum windows and ignore the partition until firmware needs it enforced.

## §4 The bus

`read/write {8,16,32}` resolve a RAM-like region on the fast path (SDRAM and
flash first — they dominate traffic), else route to `Peripherals` when the
address is in peripheral space (`0x4000_0000..0x4400_0000`), else warn-once
and read 0. Narrow (8/16-bit) peripheral IO extracts / replicates the
addressed lanes of the 32-bit register, matching rp2350-rs.

## §5 Peripheral convention

Every block exposes:

```rust
fn read(&mut self, offset: u32) -> u32;      // offset < 0x4000; may have side effects
fn write(&mut self, offset: u32, value: u32);
fn irq_pending(&self) -> bool;               // where relevant
```

The aggregate routes by 16 KiB-aligned base. There is **no** SET/CLR/TGL
alias and **no** Secure/Non-Secure mirror (that was a Series-2 EFR32 trait) —
i.MX RT peripherals are plain word-register blocks. GPIO's `DR_SET`/`DR_CLEAR`
/`DR_TOGGLE` are ordinary registers at fixed offsets, handled inside `gpio`.

Interrupt lines are assembled level-sensitively into an `IrqMask` each step
(`Peripherals::irq_lines`) and sampled by the core, so a line that deasserts
before being taken stops being pending — Armv7-M semantics.

## §6 Boot & the MadMachine two-stage model

On hardware: the boot ROM reads the FlexSPI config block + IVT at
`0x6000_0000`, runs the DCD, and jumps into a resident bootloader ("eboot").
eboot reads the `micro.img` header (load address `0x8000_0000`, SHA-256/CRC),
copies the linked Zephyr+Swift user image into SDRAM, and jumps to `__start`.

For bring-up we **short-circuit** this: load the user image directly to SDRAM
and set `VTOR = 0x8000_0000`, so `reset()` fetches SP/PC from its vector
table. Modeling eboot + SEMC/SDRAM init is M5/M8. The console is **LPUART1**.

## §7 Time

Cycle-lite: one instruction = one `cycles` tick (refined later). Time-driven
peripherals (GPT, PIT, watchdogs) advance by the retired-cycle delta each
`Rt1060::step`. No wait states, no bus contention, PLLs lock instantly.

# rt1060-rs

A cycle-lite, dependency-free **Rust emulator for the NXP i.MX RT1060**
(MIMXRT1062DVL6B, single Arm Cortex-M7). It runs **real, unmodified
firmware** — the end goal is booting **MadMachine SwiftIO** (embedded Swift)
on the **SwiftIO Micro** board, with Zephyr and Arduino images along the way.

Built in the lineage of [rp2040js](https://github.com/wokwi/rp2040js),
[rp2350-rs](https://github.com/ajsb85/rp2350-rs), and
[mg24-rs](https://github.com/ajsb85/mg24-rs): the CPU is register state only,
every memory access goes through a borrowed `Bus`, and there is no
`Rc<RefCell>`, no trait objects, and no allocation in the hot loop.

## Target hardware — MadMachine SwiftIO Micro

| | |
|---|---|
| MCU | NXP **MIMXRT1062** (i.MX RT1060) |
| Core | Arm **Cortex-M7** r1p2, FPv5-D16, L1 I/D cache, MPU |
| Clock | 600 MHz |
| RAM | 32 MB SDRAM (SEMC @ `0x8000_0000`) + 512 KB FlexRAM TCM + 1 MB OCRAM |
| Flash | 16 MB FlexSPI NOR (`0x6000_0000`) |
| I/O | 44 GPIO, 14 ADC, 14 PWM, 2 I²C, 2 SPI, 3 UART, 1 I²S, 1 CAN, SD, RGB LED |

## Status

[![SVD register coverage](https://img.shields.io/badge/SVD_coverage-42%2F50_register--complete-green)](docs/svd-coverage.md)

Milestone tracking lives in [`ROADMAP.md`](ROADMAP.md). Snapshot:

| Area | State |
|---|---|
| Cortex-M7 core (Thumb-2 + DSP parallel-add/SEL, exceptions, NVIC 158 IRQs, SysTick, MPU) | ✅ ported |
| FPv5-D16 FPU (single- **and** double-precision, GE flags) | ✅ |
| Memory map + AIPS bus (ITCM/DTCM/OCRAM/FlexSPI/SDRAM) | ✅ |
| Loader (ELF32, raw bin, MadMachine `micro.img`) | ✅ |
| LPUART (console), GPIO, CCM, GPT, SRC, WDOG | ✅ |
| CCM_ANALOG PLLs (LOCK/OSC/DCDC), SEMC status, boot spin-loops | ✅ terminate |
| RGB LED observable (GPIO1 9/10/11, active-low) via `led_rgb()` | ✅ |
| Clock-tree frequency model (core/IPG/PERCLK/UART) → `core_hz()` | ✅ |
| PIT, GPT (clock-domain accurate) | ✅ |
| eDMA (32-ch TCD through the bus) + DMAMUX hw requests, LPI2C, LPSPI, ADC | ✅ + device hooks |
| FlexPWM (`pwm_duty()`) + QTMR (PERCLK counting) | ✅ |
| USDHC + attachable **SD card** — real Zephyr driver inits it (reset, CMD0-CMD7 ident, CMD6 switches) + **mounts FAT** | ✅ |
| USB OTG register block (init runs; full CDC enumeration ⬜) | ✅ |
| FlexCAN (message buffers + loopback), SAI/I²S | ✅ |
| FlexSPI IP command engine (program/read/erase + JEDEC ID vs backed NOR) | ✅ |
| eDMA scatter-gather + channel linking; PLL2 PFD clock roots | ✅ |
| **Boots real NXP SDK blinky (ITCM / FlexSPI-XIP / SDRAM) — LED toggles** | ✅ |
| **Boots a real embedded-Swift blink — drives the SwiftIO RGB LED** | ✅ |
| **Boots the REAL MadMachine SwiftIO Blink** (`mm build`: SwiftIO + Zephyr + embedded Swift) through the full RTOS stack — **zero unimplemented instructions**, LED toggles | ✅ |
| **Runs 9 REAL MadMachine SwiftIO Playground examples** — Potentiometer (ADC+IRQ), BreathingLED (PWM), Humiture (SHT3x I2C), Accelerometer (LIS3DH I2C), RTC (PCF8563 I2C), LCD (ST7789 SPI), Speaker (SAI/I2S), SerialLEDSwitch (UART RX) — each drives a real peripheral end-to-end with an emulated device/input | ✅ |
| **Emulates `mm download` end-to-end** — boots the real `SerialLoader` bootloader, programs a real `micro.img` to the NOR `user` partition over the serial protocol, then **two-stage boots** it: `cold_boot_from_flash` models the Boot ROM (NOR → parse header → SDRAM → run the Zephyr app) | ✅ |
| **GDB remote stub** + `RT1060_TRACE` logging + criterion benches | ✅ |
| **SVD register audit** — 50 in-scope peripherals, 42 register-complete (buses/clocks/timers), critical status bits verified ([`docs/svd-coverage.md`](docs/svd-coverage.md)) | ✅ |
| SwiftIO 44-pin map (id→GPIO, from HAL static analysis) via `swiftio_pin()` | ✅ |
| **HIL parity vs a physical Teensy 4.1** (same MIMXRT1062) — boots PJRC's unmodified Arduino blinky via the i.MX RT Boot ROM → IVT path (`cold_boot_from_ivt`) and reproduces the **exact LED cadence** (SysTick 100 kHz external clock); cross-checked against the board flashed with the same `.hex` | ✅ |
| **Real MadMachine SwiftIO stack on a Teensy 4.1** — `import SwiftIO`/Zephyr re-based off SDRAM into OCRAM + wrapped as a Teensy flash image; boots with zero unimplemented instructions and blinks the onboard LED (`DigitalOut(Id.D16)`); flashed to the physical board, LED blinks at 1 Hz | ✅ |
| USB CDC enumeration, SEMC real command decode, ENET | ⬜ ROADMAP |
| MadMachine two-stage bootloader (eboot); WASM front-end; SwiftIO USB-serial HIL | ⬜ ROADMAP |

## Quick start

```rust
use rt1060_rs::{Rt1060, loader};

// Load a MadMachine user image (runs from SDRAM at 0x8000_0000).
let bytes = std::fs::read("micro.img").unwrap();
let image = loader::load_micro_img(&bytes).unwrap();
let mut soc = Rt1060::boot(&image);

// Run and read what the firmware printed on the LPUART1 console.
soc.run(1_000_000);
print!("{}", soc.console_string());
```

Raw binaries and ELFs load the same way via `loader::load_bin(addr, &data)`
and `loader::load_elf(&bytes)`.

### Debug with GDB

```bash
cargo run --example gdbserver -- firmware.elf 3333
gdb-multiarch -ex "target remote :3333" firmware.elf
```

Register/memory access, breakpoints, and step/continue all work. Set
`RT1060_TRACE=1` to log peripheral writes (`=all` for reads too) while a
firmware image brings the chip up — the fastest way to see which registers a
driver pokes.

## Build & test

```bash
cargo test                                   # unit + integration tests
cargo clippy --all-targets -- -D warnings    # lint gate
cargo fmt --all -- --check                   # format gate
```

The library is `#![no_std]`-friendly in spirit (host-side `std` for I/O
sinks) and has **zero runtime dependencies**.

## Register truth

Every nontrivial register cites its source: the CMSIS header
`MIMXRT1062.h`, the CMSIS-SVD `MIMXRT1062.xml`, the i.MX RT1060 reference
manual, or the cross-checked Renode `IMX*` models (RT1064 is memory-map
identical to RT1062). See [`CLAUDE.md`](CLAUDE.md) for the hard rules and
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the workflow.

## SVD register coverage

Audited by [`svd-coverage.py`](../../dev/boardforge/tools/svd-coverage.py)
against the NXP CMSIS-SVD `mcux-soc-svd/MIMXRT1062/MIMXRT1062.xml` (117
peripherals); scope in [`svd-scope.json`](svd-scope.json), full report in
[`docs/svd-coverage.md`](docs/svd-coverage.md).

**50 in-scope peripherals — 42 register-complete, 8 with documented surface
gaps.** The audited set is the CPU buses, clock/reset tree, timers, and the
NOR/SDRAM front-ends; the other 67 SVD peripherals (ENET, the full DMA/CAN
register maps, USBPHY, display/camera, crypto) are out-of-scope stubs.

Two backing styles, so a sub-100% number can mean two different things:

- **Explicit per-register models** — GPIO×9, LPUART×8, ADC×2, GPT×2, PIT, SRC,
  DCDC, WDOG×2/RTWDOG, and the LPI2C/LPSPI buses. Unhandled offsets truly read
  0, so the audit count is exact; **27 of these are 100%**.
- **Whole-window backing** — CCM, CCM_ANALOG (SET/CLR/TOG alias quad), SEMC,
  FLEXSPI×2, IOMUXC(_GPR), FlexPWM×4, QTMR×4. A flat offset-indexed array
  covers **every** register by construction, so the reported <100% is an audit
  artifact (no per-offset literal to match), not a missing register — the
  status bits firmware polls (`CDHIPR`, `STS0` idle, PLL `LOCK`) are forced in
  explicit arms.

Critical status contracts all verify in source: LPUART `STAT.TDRE/RDRF`, LPI2C
`MSR.NDF/RDF/MBF/SDF`, LPSPI `SR.TDF/RDF/TCF`, CCM_ANALOG PLL `LOCK` +
`OSC_XTALOK`.

### v2 register gaps (genuine, in-scope)

The only genuinely-unmodeled in-scope registers — each an unmodeled *mode* or
an *instant-transfer* timing knob with no observable effect at this emulator's
transaction fidelity (behavioural vs. surface, called out honestly):

- [ ] **LPI2C slave block** — `SCR`/`SSR`/`SIER`/`SDER`/`SCFGR1/2`/`SAMR`/
      `SASR`/`STAR`/`STDR`/`SRDR` read 0: the SwiftIO / Zephyr / Arduino
      targets drive LPI2C as **I²C master only** (*behavioural* —
      START/addr/data/STOP with host-answered reads). Slave mode is the v2
      item.
- [ ] **LPI2C master timing** — `MCFGR2` (glitch filter), `MDMR` (data-match),
      `MCCR0` (clock high/low): *register-surface* — no timing effect at
      instant transfer. v2 stores/reads-back rather than dropping the write.
- [ ] **LPSPI `DMR1` / `CCR`** — data-match 1 and the clock-delay fields
      (`SCKDIV`/`DBT`/`PCSSCK`/`SCKPCS`): *register-surface* — data-match is
      unmodeled and the shift is instantaneous. v2 is store/read-back.

Everything else the auditor flags is a whole-window readback block
(functionally complete) — see [`docs/svd-coverage.md`](docs/svd-coverage.md).

## License

MIT © 2026 Alexander Salas Bastidas `<ajsb85@firechip.dev>`. Portions ported
from mg24-rs / rp2350-rs and inspired by rp2040js (Uri Shaked, MIT).

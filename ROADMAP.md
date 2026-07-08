# ROADMAP — rt1060-rs

Milestone plan for emulating the NXP i.MX RT1060 (MIMXRT1062, SwiftIO Micro)
well enough to boot MadMachine SwiftIO firmware, then Zephyr and Arduino.
Ordered by dependency. Check items off as they land and mirror the snapshot
table in `README.md`.

Legend: ✅ done · ⏳ partial / stored-readback · ⬜ not started.

## M0 — Project bootstrap ✅

- [x] Cargo crate, MIT license headers, `.gitignore`/`.gitattributes`
- [x] tbdflow config, DoD checklist, Conventional Commits, SSH-signed commits
- [x] CLAUDE.md / CONTRIBUTING.md / README.md / ROADMAP.md / DESIGN.md
- [x] GitHub repo (Actions disabled), topics + description

## M1 — Cortex-M7 core ✅

Ported from the mg24-rs Cortex-M33 core (shared Thumb-2 mainline + DSP ISA).

- [x] Thumb-16 + Thumb-2 wide decode/execute, IT blocks, DSP/SIMD, sat
- [x] Exceptions: SVC/PendSV/SysTick/NVIC, stack framing, EXC_RETURN, nesting
- [x] **NVIC widened to 158 external IRQs** (`IrqMask`, 5×32-bit words)
- [x] SysTick, MPU (PMSAv7, 16 regions, stored-readback), CPUID = M7 r1p2
- [x] 98 instruction/exception unit tests green
- [ ] FPv5-**D16** double-precision FPU (SP path ported; DP ops + `vldm.64`
      / `vstm.64` widening still needed — SwiftIO is soft-float, so deferred)
- [ ] L1 I/D cache maintenance ops (CCR IC/DC, `mcr` cache ops) as no-ops
- [ ] Decode cache for the flash/SDRAM instruction stream (rp2350-rs precedent)

## M2 — Memory map + AIPS bus ✅

- [x] ITCM `0x0` / DTCM `0x2000_0000` / OCRAM `0x2020_0000`
- [x] FlexSPI NOR flash `0x6000_0000` (16 MB, erased 0xFF)
- [x] SEMC SDRAM `0x8000_0000` (32 MB)
- [x] AIPS peripheral routing by 16 KiB base, narrow-IO lanes, warn-once unmapped
- [ ] FlexRAM bank routing via `IOMUXC_GPR_GPR14/16/17` (TCM/OCRAM partition)
- [ ] Boot ROM stub at `0x0020_0000` (mask-ROM API entry points)

## M3 — Loader ✅

- [x] ELF32-LE (PT_LOAD segments, `e_entry`)
- [x] Raw binary at an explicit address
- [x] MadMachine `micro.img` (4 KiB header → SDRAM payload)
- [ ] `swiftio.bin` (payload + CRC32 trailer) + CRC/SHA-256 verification
- [ ] Intel HEX / SREC (NXP SDK example artifacts)

## M4 — Starter peripherals ✅ / ⏳

- [x] **LPUART** (console TX/RX FIFO, STAT/CTRL, TX/TC/RX interrupts)
- [x] **GPIO** (DR/GDIR/PSR, DR_SET/CLR/TOGGLE, combined interrupts)
- [x] **CCM** (clock gates readback, CDHIPR-not-busy)
- [x] **GPT** (free-running counter, prescaler, output compare + interrupt)
- [x] **SRC** (reset cause SRSR, boot-mode/GPR readback, SW reset request)
- [x] **WDOG1/2 + RTWDOG** (stored-readback, no bite yet)
- [x] CCM_ANALOG (PLL LOCK bits seeded), IOMUXC, GPC, SNVS, DCDC, OCOTP, PIT
      as stored-readback `RawRegs`

## M5 — Clock tree & board bring-up ⏳

Boot-time spin-loops all terminate (verified end-to-end in `tests/bringup.rs`
against the exact poll bits `fsl_clock.c` / `fsl_semc.c` / `clock_config.c`
use). Frequency math and the full pin table remain.

- [x] CCM_ANALOG model: SET/CLR/TOG register-quad aliases + forced PLL LOCK
      (bit 31, every PLL), MISC0 `OSC_XTALOK` (bit 15), `LOWPWR_CTRL`
      `XTALOSC_PWRUP_STAT` (bit 16)
- [x] CCM handshake: CDHIPR reads not-busy so `CLOCK_SetDiv`/`SetMux` exit
- [x] DCDC `REG0.STS_DC_OK` (bit 31) forced so the VDD_SOC ramp poll exits
- [x] SEMC status: `MCR.SWRST` self-clear, `INTR.IPCMDDONE` on keyed IPCMD,
      `STS0.IDLE`; SDRAM stays pre-mapped RAM
- [x] IOMUXC pad mux (stored-readback) + observable **RGB LED** = GPIO1
      pins 9/10/11 (pads `GPIO_AD_B0_09/10/11`, active-low) via `led_rgb()`
- [x] CCM_ANALOG PLL **frequency** math (ARM PLL 600 MHz path exact; SYS/USB/
      PFD nominal) → `clocks.rs`; `Rt1060::core_hz()`/`perclk_hz()`
- [x] CCM clock roots (core/AHB/IPG/PERCLK/UART) feeding GPT/PIT time base
      via a per-domain fractional cycle converter in `Peripherals::tick`
- [ ] PLL2/PLL3 PFD fractional-divider math (currently nominal frequencies)
- [x] SwiftIO Micro hardware pin assignments transcribed from the Zephyr
      board `../zephyr/boards/arm/mm_feather/{mm_feather.dts,pinmux.c}`
      (RGB LED, console LPUART1, I²C1/3, LPSPI3/4, SD/USDHC1 + card-detect,
      SAI1/I²S, FlexPWM/GPT/ADC/USB1) → `board::pinmux`
- [ ] SwiftIO *logical* id `D0..D43` → pad ordering: only in the prebuilt
      HalSwiftIO `swifthal_gpio_open` driver + the SwiftIOPinout image; a thin
      translation layer over the (now-mapped) hardware pins — do not guess
- [ ] SEMC real command decode / SDRAM refresh timing (beyond status)

## M6 — DMA & serial/analog peripherals ⏳

- [x] **eDMA** transfer engine (32-ch TCD, minor/major loop, INT/DONE, IRQ)
      driven through the system bus; software START + action registers
- [x] **PIT** full 4-channel model (cascade, PERCLK, IRQ 122)
- [x] **LPI2C** ×2 master with a pluggable `I2cDevice` hook (+ `MemI2cDevice`)
- [x] **LPSPI** ×2 full-duplex master with an `SpiDevice` hook (+ `SeqSpiDevice`)
- [x] **ADC** ×2 (12-bit, programmable channel inputs, COCO/AIEN IRQ 67/68)
- [x] DMAMUX request routing + hardware-request triggering (source→channel)
- [x] PWM: FlexPWM (duty observability) + **QTMR** (PERCLK counting, IRQ)
- [ ] SAI/I²S, FlexCAN
- [ ] eDMA scatter-gather (ESG), channel linking, error reporting

## M7 — Storage, USB, connectivity ⏳

- [x] **USDHC + SD card image** (attach a card; SD init + block read/write
      through the PIO data port; SwiftIO loads apps/assets from SD)
- [x] **USB OTG** controller register block (USB1/USB2; reset self-clear,
      PORTSC connected, USBSTS W1C) — init runs without hanging
- [ ] USB device transfer engine (queue heads / dTDs) + CDC-ACM enumeration
      (the `mm download` / console bridge)
- [ ] USDHC ADMA/DMA data path (currently PIO)
- [ ] FlexSPI controller register model + XIP program/erase
- [ ] ENET, CAN (FlexCAN), I²S (SAI)

## M8 — Boot real firmware ⏳

- [x] **Boot a real, unmodified NXP SDK Cortex-M7 image** (RT1050 EVK LED
      blinky, ITCM `0x2000`): runs clock/pin-mux/GPIO init with **zero
      unmapped-register / unimplemented-instruction hits** and **blinks the
      LED** (`GPIO1_IO09` toggles) — `tests/boot_fixture.rs`, `examples/probe.rs`
- [ ] Boot the MadMachine bootloader ("eboot") from FlexSPI flash
- [ ] Two-stage boot: verify `micro.img`, copy to SDRAM, jump to `__start`
- [ ] Bring Zephyr up to `PRE_KERNEL → APPLICATION`; assert console banner
- [ ] Run the SwiftIO `01LED` / `Blink` example; assert RGB LED toggling
- [ ] HIL parity: compare against a physical SwiftIO Micro over USB-serial

## M9 — Tooling ⏳

- [x] GDB remote-serial stub: breakpoints, mem, regs, step/continue,
      Ctrl-C; `examples/gdbserver.rs` + `gdb-multiarch`
- [x] Trace/log gating by env var (`RT1060_TRACE` = writes, `=all` = reads)
- [x] `criterion` benchmarks (`benches/step.rs`): ALU-loop + real-firmware
      instruction throughput; sustained ~13M inst/s confirms the hot loop
      allocates nothing per step
- [ ] WASM front-end (`wasm-bindgen`) for an in-browser SwiftIO playground
- [ ] Hardware breakpoints (FPB) + watchpoints in the GDB stub

## Known simplifications (revisit as firmware demands)

- Caches are coherent no-ops; no wait states or bus contention (cycle-lite).
- Watchdogs never bite; PLLs lock instantly; SDRAM is pre-mapped RAM.
- The `unknown` peripheral fallback aliases distinct unmodeled blocks onto
  one page — every first touch logs, forming the TODO list for M5–M7.
- SAU registers in the core are vestigial (M7 is Armv7E-M, no TrustZone).

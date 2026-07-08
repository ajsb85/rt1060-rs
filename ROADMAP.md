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
- [x] **DSP parallel add/subtract + SEL + APSR.GE** (SADD8/UADD8/USUB8/
      UADD16/ASX/SAX and Q/H variants — the SDK memcpy uses uadd8+sel)
- [x] Exceptions: SVC/PendSV/SysTick/NVIC, stack framing, EXC_RETURN, nesting
- [x] **NVIC widened to 158 external IRQs** (`IrqMask`, 5×32-bit words)
- [x] SysTick, MPU (PMSAv7, 16 regions, stored-readback), CPUID = M7 r1p2
- [x] instruction/exception unit tests green (incl. DSP + DP-FP)
- [x] FPv5-**D16** double-precision FPU (D0..D15 alias the S pairs; VADD/VSUB/
      VMUL/VDIV/VABS/VNEG/VSQRT/VCMP.F64 + cross-precision/integer VCVTs —
      hit by `CLOCK_GetPllFreq`'s f64 (a*b)/c even in soft-float firmware)
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
- [x] SwiftIO *logical* id 0..47 (`D0..D43` + RGB/DL) → `(GPIO, pin)`:
      **recovered by static analysis** of the HalSwiftIO `gpio_pin_maps` table
      (`swift_gpio.c.obj` in `libapp.a`) and cross-validated against the
      devicetree → `board::SWIFTIO_PIN_MAP`, `Rt1060::swiftio_pin(id)`
- [ ] SEMC real command decode / SDRAM refresh timing (beyond status)

## M6 — DMA & serial/analog peripherals ⏳

- [x] **eDMA** transfer engine (32-ch TCD, minor/major loop, INT/DONE, IRQ)
      driven through the system bus; software START + action registers
- [x] **PIT** full 4-channel model (cascade, PERCLK, IRQ 122)
- [x] **LPI2C** ×2 master with a pluggable `I2cDevice` hook (+ `MemI2cDevice`)
- [x] **LPSPI** ×4 full-duplex master with an `SpiDevice` hook (+ `SeqSpiDevice`);
      SwiftIO `Id.SPI0` is LPSPI3 — the real ST7789 LCD renders over it
- [x] **ADC** ×2 (12-bit, programmable channel inputs, COCO/AIEN IRQ 67/68)
- [x] DMAMUX request routing + hardware-request triggering (source→channel)
- [x] PWM: FlexPWM (duty observability) + **QTMR** (PERCLK counting, IRQ)
- [ ] SAI/I²S, FlexCAN
- [x] **LPI2C ×4 + interrupt-driven completion** — SwiftIO `Id.I2C0` is
      **LPI2C3** (`0x403F_8000`, IRQ 30); adding LPI2C3/4 lets the Zephyr
      `i2c_mcux_lpi2c` non-blocking driver's IRQ state machine complete, so the
      real Humiture/SHT3x example reads the sensor over the interrupt path
- [ ] eDMA scatter-gather (ESG), channel linking, error reporting

## M7 — Storage, USB, connectivity ⏳

- [x] **USDHC + SD card image** (attach a card; SD init + block read/write
      through the PIO data port; SwiftIO loads apps/assets from SD)
- [x] **USB OTG** controller register block (USB1/USB2; reset self-clear,
      PORTSC connected, USBSTS W1C) — init runs without hanging
- [ ] USB device transfer engine (queue heads / dTDs) + CDC-ACM enumeration
      (the `mm download` / console bridge)
- [ ] USDHC ADMA/DMA data path (currently PIO)
- [x] **FlexSPI IP command engine** — LUT/IPCR/IPCMD driving program / read /
      sector-erase + JEDEC ID against the backed NOR (Zephyr littlefs/NOR)
- [ ] ENET, CAN (FlexCAN), I²S (SAI)

## M8 — Boot real firmware ✅

- [x] **Boot a real, unmodified NXP SDK Cortex-M7 image** (RT1050 EVK LED
      blinky, ITCM `0x2000`): runs clock/pin-mux/GPIO init with **zero
      unmapped-register / unimplemented-instruction hits** and **blinks the
      LED** (`GPIO1_IO09` toggles) — `tests/boot_fixture.rs`, `examples/probe.rs`
- [x] **Boot the REAL MadMachine SwiftIO Blink** (`mm build`, SDK 2.2.0 — full
      SwiftIO + Zephyr + embedded Swift, from SDRAM) end-to-end with **zero
      unimplemented instructions**
- [x] Bring Zephyr up through kernel + device init to the application; assert
      the LittleFS console log over LPUART1
- [x] Run the SwiftIO `Blink` example; **assert the RGB LED toggles** (RED/BLUE
      at the `sleep(ms:500)` interval) by SwiftIO logical id
- [x] Run **9 real SwiftIO Playground examples** end-to-end (ADC/PWM/I2C-sensors/
      SPI-LCD/I2S/UART-RX), each driving a real peripheral with an emulated
      device/input — `tests/boot_fixture.rs`
- [x] **Boot the MadMachine `SerialLoader` recovery bootloader** (loads to ITCM,
      brings up the full Zephyr stack + littlefs, logs "Recovery base Zephyr!") —
      the prerequisite for emulating `mm download`
- [x] **`mm download` over the framed serial protocol** — drive the real
      SerialLoader bootloader's download protocol (`PREAMBLE | tag | length |
      payload | crc32`) over LPUART1: SYNC + a RAM download lands its bytes in
      SDRAM, verified (`tests/mm_download.rs`, `examples/mm_download.rs`). No USB
      stack needed — the bootloader accepts the download over UART
- [x] **FLASH/PARTITION download + two-stage boot** — `mm download` a real
      `micro.img` to the NOR `user` partition (`PART_BEGIN/DATA/END/SETBOOT`),
      then read it back from NOR, parse the header, load the payload to SDRAM
      and boot it into the Zephyr app (`tests/mm_download.rs`)
- [x] **In-emulator two-stage boot** — `Rt1060::cold_boot_from_flash` models the
      Boot ROM / first-stage loader: read the flashed `micro.img` from NOR, stage
      the payload to SDRAM, reset into it (the SoC boots the deployed image, not
      the harness). The SerialLoader's own `EXECUTE`/boot path is blocked (rejects
      an SDRAM target `0xff`; the SDK ships no `eboot` first-stage binary)
- [x] **FS_FILE download tags** — `mm copy` a file to the on-board littlefs
      (`fs_file_begin(path)`/`data`/`end`); the bootloader writes it to `/lfs` on
      the NOR and the content lands in the littlefs partition (`tests/mm_download.rs`)
- [x] **HIL parity vs a physical Teensy 4.1** (same MIMXRT1062 silicon) — boot
      PJRC's unmodified Arduino `blink_fast_Teensy41` (a completely independent
      firmware stack: Teensyduino core, not MadMachine/Zephyr) through the i.MX
      RT **Boot ROM → IVT** path (`Rt1060::cold_boot_from_ivt`), with **zero
      unimplemented instructions**, and reproduce the **exact hardware LED
      cadence** — `delay(100)` timed by SysTick off its 100 kHz external
      reference clock. Cross-checked against the physical board flashed with the
      identical `.hex` via `teensy_loader_cli`. Surfaced + fixed two parity
      bugs: SysTick ignored `CSR.CLKSOURCE` (external-clock timing), and CCM
      `CBCMR` wasn't seeded to its reset value (core clock mis-resolved to PLL2
      528 MHz instead of the ARM PLL 396 MHz). `tests/teensy_hil.rs`,
      `examples/teensy.rs`
- [ ] HIL parity: compare against a physical SwiftIO Micro over USB-serial;
      SwiftIO `11WiFi` (SPI+ESP32)

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

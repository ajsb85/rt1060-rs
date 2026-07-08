# Test fixtures

Real firmware binaries used by the integration tests. Every fixture must be a
real hardware artifact or an unmodified public release, with provenance
recorded here. If the emulator diverges from a physical SwiftIO Micro running
the same fixture, the emulator is wrong.

## Candidates (to be added as milestones land)

- **MadMachine SwiftIO examples** — build `../Template/Examples/Blink` or
  `../MadExamples/Examples/SwiftIOPlayground/01LED` with the `mm` toolchain;
  the `micro.img` (SwiftIO Micro) loads to SDRAM `0x8000_0000`.
- **SerialLoader.bin** — `../mm-sdk/boards/SerialLoader.bin`, the MadMachine
  second-stage RAM loader (initial SP `0x2002_b238`, reset `0x0000_4138`;
  runs from ITCM `0x0`). Good early CPU/vector-table bring-up fixture.
- **NXP SDK LED blink** — `../mcu-boot-utility/apps/.../led_blinky_0x*_iar.elf`
  (RT1050 ≈ RT1062) at ITCM `0x0000_2000` or FlexSPI `0x6000_2000`.

## Committed fixtures

### `rt1050_led_blinky_itcm.elf`

A real, **unmodified** NXP MCUXpresso SDK LED-blinky for the i.MX RT1050 EVK,
IAR-built to run from ITCM at `0x0000_2000` (vector table there; reset handler
`0x0000_4fc0`, initial SP `0x2002_0000`). Copied verbatim from
`../mcu-boot-utility/apps/NXP_MIMXRT1050-EVKB_Rev.A/led_blinky_0x00002000_iar.elf`.

RT1050 and RT1062 share the Cortex-M7 core and are register-compatible for the
blocks this image touches (CCM clocks, IOMUXC, GPIO). Exercised by
`tests/boot_fixture.rs`:

- it boots and runs its clock/pin-mux/GPIO init with **zero unmapped-register
  or unimplemented-instruction hits**, and configures the EVK user LED
  (`GPIO1_IO09`) as an output (fast test);
- run long enough, `GPIO1_IO09` **toggles** — the LED blinks (the `--ignored`
  deep test, ~250M instructions).

### `rt1050_led_blinky_flexspi.elf` / `rt1050_led_blinky_sdram.elf`

The same SDK blinky linked to run **XIP from FlexSPI NOR** (`0x6000_2000`) and
**from SDRAM** (`0x8000_2000`, the MadMachine run location). Both boot, run
init cleanly, and blink `GPIO1_IO09` — proving the flash-XIP and SDRAM code
paths. From the same SDK app dir as the ITCM variant.

Still available (not committed): `../mm-sdk/boards/SerialLoader.bin` (the
MadMachine second-stage RAM loader).

### `swiftio_blink_embedded_swift.elf`

A **real embedded-Swift** program (not the full MadMachine Zephyr stack — that
needs the `madmachine-sdk` Swift SDK artifactbundle, which isn't installed
here). Source in `swiftio_blink_src/`:

- `blink.swift` — mirrors the MadMachine `Blink` example (toggle RED + BLUE)
  but pokes the GPIO1 registers directly via `UnsafeMutablePointer`, using the
  RGB LED pins recovered from the HalSwiftIO binary (RED = GPIO1 pin 9,
  BLUE = GPIO1 pin 11).
- `startup.c` — Cortex-M vector table (`[SP, Reset_Handler]`) + bare-metal
  stubs for the runtime symbols embedded Swift references.
- `link.ld` — links it into SDRAM at `0x8000_0000` (the MadMachine run
  location).

Built with the installed Swift 6.2 embedded toolchain:

```sh
swiftc -target armv7em-none-none-eabi -enable-experimental-feature Embedded \
       -parse-as-library -Onone -wmo -c blink.swift -o blink.o
arm-none-eabi-gcc -mcpu=cortex-m7 -mthumb -c startup.c -o startup.o
arm-none-eabi-gcc -mcpu=cortex-m7 -mthumb -nostdlib -nostartfiles \
       -T link.ld startup.o blink.o -o swiftio_blink_embedded_swift.elf
```

`tests/boot_fixture.rs::embedded_swift_blinks_the_swiftio_rgb_led` boots it and
watches the RED/BLUE pins toggle via `Rt1060::swiftio_pin(id)`.

### `madmachine_swiftio_blink.elf`

The **real, unmodified MadMachine SwiftIO Blink** — the canonical `Blink`
example (`red.toggle(); blue.toggle(); sleep(ms: 500)`) built with the
**MadMachine SDK 2.2.0** via `mm build` for the SwiftIO Micro: the full
embedded-Swift `SwiftIO` + `MadBoards` package linked against the prebuilt
Zephyr RTOS + HAL, running from SDRAM `0x8000_0000`.

It boots through the entire Zephyr kernel and device init in the emulator —
clocks, ADC (auto-calibration), GPT, LPUART console, LPI2C, LPSPI, and the
FlexSPI/littlefs storage layer — and runs the Blink loop, toggling the RGB LED
(RED = GPIO1 pin 9, BLUE = pin 11) at the 500 ms interval. `RT1060_TRACE` /
`examples/probe.rs` show the live Zephyr console log. See
`tests/boot_fixture.rs`.

### `madmachine_swiftio_pot.elf`

The **real MadMachine Potentiometer example** (SwiftIO Playground `04Potentiometer`)
built with the MadMachine SDK 2.2.0: `AnalogIn(Id.A0).readVoltage()` printed once
a second. Unlike Blink (GPIO + console only) it drives the **ADC** and the full
**external-interrupt** path — the ADC conversion-complete IRQ routes through
Zephyr's `_isr_wrapper`, which reads `IPSR` to index the software ISR table.
Finding this exposed a core bug (`mrs Rd, IPSR` returned 0, sending every
external interrupt to a garbage handler). Driving the ADC inputs to a known
value makes the firmware print the matching voltage. See `tests/boot_fixture.rs`.

### `madmachine_swiftio_pwm.elf`

The **real MadMachine BreathingLED example** (SwiftIO Playground
`03Buzzer/BreathingLED`) built with the MadMachine SDK: a `PWMOut(Id.PWM4A)`
whose duty cycle ramps 0→1→0 to "breathe" an LED. It drives the FlexPWM
peripheral; the emulator observes the changing duty via `Rt1060::pwm_duty(4, 3,
Chan::A)`. See `tests/boot_fixture.rs`.

### `madmachine_swiftio_i2c.elf`

The **real MadMachine Humiture example** (SwiftIO Playground `05Humiture`) built
with the MadMachine SDK: the MadDrivers `SHT3x` driver over `I2C(Id.I2C0)`
reading temperature + humidity. SwiftIO `Id.I2C0` is **LPI2C3** (`0x403F_8000`).
The Zephyr `i2c_mcux_lpi2c` driver is interrupt-driven
(`LPI2C_MasterTransferNonBlocking` + `k_sem_take`, completed by the IRQ-30 ISR
state machine), so this fixture exercises the whole LPI2C interrupt path plus a
real sensor driver. `tests/boot_fixture.rs` attaches an emulated SHT3x and
asserts the decoded temperature is printed.

### `madmachine_swiftio_lcd.elf`

The **real MadMachine LCD example** (SwiftIO Playground `08LCD`) built with the
MadMachine SDK: the MadDrivers `ST7789` driver over `SPI(Id.SPI0)` filling a
240×240 display with solid colours. SwiftIO `Id.SPI0` is **LPSPI3**
(`0x4039_C000`, IRQ 34); the driver is interrupt-driven
(`LPSPI_MasterTransferNonBlocking` + `spi_context_wait`). `tests/boot_fixture.rs`
attaches a recording display and asserts a full red screen (57 600 `0xF8`
pixel bytes) is written over SPI.

### `madmachine_swiftio_uart.elf`

The **real MadMachine SerialLEDSwitch example** (SwiftIO Playground
`10UART/SerialLEDSwitch`): reads `UART(Id.UART0)` and turns LED `D18` on for
`"1"`, off for `"0"`. SwiftIO `Id.UART0` is **LPUART2**; the RX path is
interrupt-driven (RDRF → IRQ 21 ISR → ring buffer). `tests/boot_fixture.rs`
pushes bytes into the LPUART2 RX FIFO and asserts the LED (`swiftio_pin(18)`)
follows — the first real-firmware validation of serial **input**.

### `madmachine_swiftio_i2s.elf`

The **real MadMachine Speaker example** (SwiftIO Playground `09Speaker`):
`I2S(Id.I2S0)` plays a musical scale of square-wave tones. SwiftIO `Id.I2S0` is
**SAI1**; the transfer is interrupt-driven (`SAI_TransferSendNonBlocking` +
`SAI_TransferTxHandleIRQ`, IRQ 56). `tests/boot_fixture.rs` captures the
transmitted SAI words and asserts a real (mostly non-zero) audio stream flows.

### `madmachine_swiftio_accel.elf` / `madmachine_swiftio_rtc.elf`

Two more real SwiftIO Playground examples over I2C (`Id.I2C0` = LPI2C3):
`07Accelerometer` (MadDrivers `LIS3DH`, register-addressed reads with the `0x80`
auto-increment bit) and `06RTC/ReadingTime` (MadDrivers `PCF8563`, a time
write→read round-trip). `tests/boot_fixture.rs` models each with a seeded
`MemI2cDevice` and asserts the decoded reading (Z = +1 g; `2023/04/09 … 10:26`).

### `mm_serial_loader.bin`

The **real MadMachine `SerialLoader` recovery bootloader** (`mm-sdk 2.2.0`,
`boards/SerialLoader.bin`) — a Zephyr-based loader that runs from ITCM and
handles `mm download`. Loaded at `0x0`, it boots cleanly (full Zephyr + littlefs,
logs "Recovery base Zephyr! mm_feather" on LPUART2) and accepts the framed
serial download protocol on LPUART1. `tests/mm_download.rs` and
`examples/mm_download.rs` drive that protocol (SYNC + RAM download) and verify
the bytes land in SDRAM — the emulated equivalent of `mm download <image>`.

### `madmachine_swiftio_blink.img`

The Blink `micro.img` (mm SDK `mm build` output for `DemoBlink`) — a 4 KiB
header (`image.py`: CRC, offset, size, load address `0x8000_0000`, SHA-256) plus
the raw SDRAM payload. `tests/mm_download.rs` programs it to the NOR `user`
partition over the `mm download` partition protocol, then two-stage boots it
(read from NOR → parse header → load to SDRAM → run) into the Zephyr app.

### `teensy41_blink_fast.elf` / `teensy41_blink_fast.hex`

**HIL parity fixture** — PJRC's own unmodified `blink_fast` example built for
the **Teensy 4.1** (from `https://www.pjrc.com/teensy/blink_both.zip`,
`blink_fast_Teensy41.{elf,hex}`, copied verbatim). The Teensy 4.1 is the same
NXP **MIMXRT1062** silicon as the SwiftIO Micro, so it cross-checks the emulated
CPU + clock + GPIO against real hardware with a completely independent firmware
stack (the Arduino/Teensyduino core, not MadMachine/Zephyr).

The Arduino sketch is `pinMode(13, OUTPUT)` then toggle pin 13 with
`delay(100)`. It boots via the i.MX RT **Boot ROM → IVT** path
(`Rt1060::cold_boot_from_ivt`): a FlexSPI config block at `0x6000_0000`, an IVT
at `0x6000_1000` whose entry is the `naked` `ResetHandler` (`0x6000_1030`) that
reconfigures FlexRAM, copies `.text.itcm` into ITCM, runs the clock tree to
F_CPU, and reaches the Arduino runtime. Pin 13 = pad `GPIO_B0_03`, remapped by
the core to the high-speed **GPIO7** bit 3 (`IOMUXC_GPR_GPR27`).

`tests/teensy_hil.rs`:

- boots with **zero unimplemented instructions**, runs from ITCM, and resolves
  **`core_hz` = 396 MHz** — the exact frequency this build programs (`PLL_ARM`
  `DIV_SELECT = 66` → VCO 792 MHz ÷ 2 ÷ `ARM_PODF` 2). This surfaced a clock
  bug: the emulator wasn't seeding CCM `CBCMR` to its reset value, so
  `PRE_PERIPH_CLK_SEL` (which the firmware leaves at the PLL_ARM reset default)
  resolved to PLL2 = 528 MHz instead of the ARM PLL (fast test);
- run long, pin 13 toggles at the **exact hardware cadence**: a first toggle at
  ~300 ms (Teensyduino `TEENSY_INIT_USB_DELAY`), then every 100 ms
  (`delay(100)`), timed by SysTick off its 100 kHz external reference clock —
  the `--ignored` deep test.

**Cross-check against the physical board:** flash the identical `.hex` with
`teensy_loader_cli --mcu=TEENSY41 teensy41_blink_fast.hex` (tools installed at
`/opt/teensy-tools`, udev rules from `pjrc.com/teensy/00-teensy.rules`) and the
onboard LED blinks at 5 Hz — the same cadence the emulator reproduces from the
same build.

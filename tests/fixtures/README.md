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

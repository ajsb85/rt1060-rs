// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Alexander Salas Bastidas <ajsb85@firechip.dev>

//! Boot **real, unmodified** NXP SDK Cortex-M7 firmware and confirm the
//! emulator runs it from every code region — the M8 milestone. The fixtures
//! are IAR-built LED blinkies for the i.MX RT1050 EVK (RT1050 ≈ RT1062: same
//! core, register-compatible peripherals) linked for three different memories:
//! ITCM (`0x2000`), FlexSPI NOR XIP (`0x6000_2000`), and SDRAM (`0x8000_2000`,
//! the MadMachine run location). Provenance: `tests/fixtures/README.md`.

use rt1060_rs::cortex_m::BreakCause;
use rt1060_rs::{Rt1060, loader};

const ITCM: &[u8] = include_bytes!("fixtures/rt1050_led_blinky_itcm.elf");
const FLEXSPI: &[u8] = include_bytes!("fixtures/rt1050_led_blinky_flexspi.elf");
const SDRAM: &[u8] = include_bytes!("fixtures/rt1050_led_blinky_sdram.elf");

/// Boot `elf`, run its init, and assert it reached GPIO setup without hitting
/// an unimplemented instruction — from whichever memory it is linked for.
fn boots_and_configures_led(elf: &[u8], expected_base: u32) {
    let image = loader::load_elf(elf).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    assert_eq!(image.base, expected_base, "linked load base");
    assert_eq!(soc.core.regs[13], 0x2002_0000, "initial SP");

    for _ in 0..2_000_000 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!("unimplemented instruction {hw:#06x} booting from {expected_base:#010x}");
        }
    }
    assert!(
        soc.bus.periph.gpio[0].is_output(9),
        "EVK user LED (GPIO1_IO09) configured as output from base {expected_base:#010x}"
    );
}

#[test]
fn boots_from_itcm() {
    boots_and_configures_led(ITCM, 0x0000_2000);
}

#[test]
fn boots_from_flexspi_nor_xip() {
    boots_and_configures_led(FLEXSPI, 0x6000_2000);
}

#[test]
fn boots_from_sdram() {
    boots_and_configures_led(SDRAM, 0x8000_2000);
}

/// Boot a **real embedded-Swift** program (compiled with Swift 6.2 for
/// `armv7em-none-none-eabi`, linked bare-metal for the RT1062) that blinks the
/// SwiftIO RGB LED, and observe it toggle the RED + BLUE pins **by their
/// SwiftIO logical ids** — end-to-end proof that Swift runs on the emulated
/// chip and drives the pins recovered from the HalSwiftIO binary. Build
/// sources: `tests/fixtures/swiftio_blink_src/`.
#[test]
fn embedded_swift_blinks_the_swiftio_rgb_led() {
    const SWIFT_BLINK: &[u8] = include_bytes!("fixtures/swiftio_blink_embedded_swift.elf");
    let image = loader::load_elf(SWIFT_BLINK).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    assert_eq!(
        image.base, 0x8000_0000,
        "runs from SDRAM (MadMachine location)"
    );
    assert_eq!(soc.core.regs[13], 0x2002_0000, "initial SP = top of DTCM");

    // RED = id 44 (GPIO1 pin 9), BLUE = id 46 (GPIO1 pin 11) — the Swift code
    // drives both together. Count RED transitions via the swiftio_pin id API.
    let mut transitions = 0;
    let mut last = soc.swiftio_pin(44);
    for _ in 0..300_000 {
        soc.step();
        let red = soc.swiftio_pin(44);
        assert_eq!(
            red,
            soc.swiftio_pin(46),
            "Swift drives RED and BLUE together"
        );
        if red != last {
            transitions += 1;
            last = red;
        }
    }
    assert!(
        transitions >= 4,
        "the Swift blink toggled the RGB LED (saw {transitions})"
    );
    // GREEN (id 45, GPIO1 pin 10) is never configured/driven by this program.
    assert_eq!(soc.swiftio_pin(45), Some(false), "GREEN untouched");
}

/// The **real, unmodified MadMachine SwiftIO Blink** — built with the
/// MadMachine SDK 2.2.0 (`mm build`) as the full SwiftIO + Zephyr + embedded-
/// Swift image, running from SDRAM. This fast check boots it through the whole
/// Zephyr kernel + device init (clocks, ADC calibration, GPT, LPUART, LPI2C,
/// LPSPI, FlexSPI/littlefs) and asserts it reached Zephyr's console logging —
/// proof the real firmware runs. Provenance: `tests/fixtures/README.md`.
#[test]
fn madmachine_swiftio_blink_boots_zephyr() {
    const BLINK: &[u8] = include_bytes!("fixtures/madmachine_swiftio_blink.elf");
    let image = loader::load_elf(BLINK).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    assert_eq!(image.base, 0x8000_0000, "MadMachine image runs from SDRAM");

    for _ in 0..12_000_000 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!("unimplemented instruction {hw:#06x} in MadMachine boot");
        }
    }
    // Zephyr's console (LPUART1) logged the littlefs bring-up — the kernel and
    // device init are running.
    let console = soc.console_string();
    assert!(
        console.contains("LittleFS"),
        "expected Zephyr console output, got: {console:?}"
    );
    // The FlexSPI IP flash engine is correct end-to-end: littlefs formats and
    // mounts on the emulated NOR (an earlier bug wrote the superblock to the
    // wrong address, so the mount failed "Superblock unwritable").
    assert!(
        console.contains("/lfs mounted"),
        "littlefs should format and mount on the emulated FlexSPI NOR"
    );
}

/// Deep check: run the real MadMachine Blink long enough for the `sleep(ms:500)`
/// loop to toggle the RGB LED — RED (id 44, GPIO1 pin 9) and BLUE (id 46, pin
/// 11). Ignored by default (~1 billion instructions through the full Zephyr
/// stack); run with `cargo test --release -- --ignored`.
#[test]
#[ignore = "runs ~1B instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_swiftio_blink_toggles_the_led() {
    const BLINK: &[u8] = include_bytes!("fixtures/madmachine_swiftio_blink.elf");
    let image = loader::load_elf(BLINK).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();

    let mut transitions = 0;
    let mut last = soc.swiftio_pin(44); // RED
    for _ in 0..1_100_000_000u64 {
        soc.step();
        let red = soc.swiftio_pin(44);
        if red != last {
            transitions += 1;
            last = red;
            if transitions >= 3 {
                break;
            }
        }
    }
    assert!(
        transitions >= 3,
        "the MadMachine Blink should toggle the RGB LED (saw {transitions})"
    );
}

/// The **real MadMachine RTC example** (`06RTC/ReadingTime`): the MadDrivers
/// `PCF8563` driver over `I2C(Id.I2C0)` sets the clock to 2023-04-09 10:26:00
/// then reads it back and prints it. A blank register-file `MemI2cDevice` at
/// 0x51 stores the time write and returns it on read (an I2C write→read
/// round-trip). Ignored by default.
#[test]
#[ignore = "runs ~40M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_rtc_round_trips_the_time_over_i2c() {
    use rt1060_rs::peripherals::lpi2c::MemI2cDevice;
    const RTC: &[u8] = include_bytes!("fixtures/madmachine_swiftio_rtc.elf");
    let image = loader::load_elf(RTC).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    // PCF8563 at 0x51 on I2C0 (LPI2C3 = index 2). Seed the VL bit (bit 7 of the
    // seconds register 0x02) so the driver's lostPower() check is true and it
    // actually writes the time before reading it back.
    soc.bus.periph.lpi2c[2].attach(Box::new(MemI2cDevice::new(0x51).with(0x02, 0x80)));
    let mut console = String::new();
    for _ in 0..60_000_000u64 {
        soc.step();
        console.push_str(&soc.console_string());
        // "10:26" is printed after the date on the same line, so waiting for it
        // guarantees the whole "2023/04/09 … 10:26:00" line has flushed.
        if console.contains("10:26") {
            break;
        }
    }
    assert!(
        console.contains("2023/04/09") && console.contains("10:26"),
        "the RTC should read back the time it set: {console:?}"
    );
}

/// The **real MadMachine Accelerometer example** (`07Accelerometer`): the
/// MadDrivers `LIS3DH` driver over `I2C(Id.I2C0)` reading X/Y/Z and printing
/// them. Unlike the SHT3x (command-based), the LIS3DH is register-addressed
/// (WHO_AM_I = 0x33, `OUT_*` at `0x28`, auto-increment bit `0x80`), so a seeded
/// `MemI2cDevice` models it: Z = +1g (raw 15987 = `0x3E73`, so `0x73`/`0x3E`
/// land at `0xAC`/`0xAD` under the auto-increment offset). Ignored by default.
#[test]
#[ignore = "runs ~40M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_accelerometer_reads_the_i2c_sensor() {
    use rt1060_rs::peripherals::lpi2c::MemI2cDevice;
    const ACCEL: &[u8] = include_bytes!("fixtures/madmachine_swiftio_accel.elf");
    let image = loader::load_elf(ACCEL).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    // LIS3DH at 0x18 on I2C0 (LPI2C3 = index 2).
    let lis3dh = MemI2cDevice::new(0x18)
        .with(0x0F, 0x33) // WHO_AM_I
        .with(0xAC, 0x73) // OUT_Z_L  (0x2C | 0x80 auto-increment)
        .with(0xAD, 0x3E); // OUT_Z_H → Z raw = 0x3E73 = 15987 → 1.0 g
    soc.bus.periph.lpi2c[2].attach(Box::new(lis3dh));
    let mut console = String::new();
    for _ in 0..60_000_000u64 {
        soc.step();
        console.push_str(&soc.console_string());
        if console.contains("z: 1.0") {
            break;
        }
    }
    assert!(
        console.contains("z: 1.0"),
        "the accelerometer should read Z = +1g: {console:?}"
    );
}

/// The **real MadMachine Speaker example** (`09Speaker`): `I2S(Id.I2S0)` plays a
/// musical scale (square-wave tones). SwiftIO `Id.I2S0` is SAI1; the transfer is
/// interrupt-driven (`SAI_TransferSendNonBlocking` + `SAI_TransferTxHandleIRQ`
/// on IRQ 56). The emulator captures the transmitted SAI words — a real audio
/// stream, not silence. Ignored by default.
#[test]
#[ignore = "runs ~40M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_speaker_streams_audio_over_i2s() {
    const I2S: &[u8] = include_bytes!("fixtures/madmachine_swiftio_i2s.elf");
    let image = loader::load_elf(I2S).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    let mut words = 0usize;
    let mut nonzero = 0usize;
    for _ in 0..40 {
        for _ in 0..1_000_000u64 {
            soc.step();
        }
        for w in soc.bus.periph.sai[0].take_output() {
            words += 1;
            if w != 0 {
                nonzero += 1;
            }
        }
    }
    assert!(
        words > 10_000,
        "the speaker should stream audio samples over I2S (saw {words})"
    );
    assert!(
        nonzero > words / 2,
        "the samples should be a real waveform, not silence ({nonzero}/{words} non-zero)"
    );
}

/// The **real MadMachine SerialLEDSwitch example** (`10UART/SerialLEDSwitch`):
/// reads `UART(Id.UART0)` and turns LED `D18` on for `"1"`, off for `"0"`.
/// SwiftIO `Id.UART0` is LPUART2; the SwiftIO HAL RX path is interrupt-driven
/// (RDRF → IRQ 21 ISR fills a ring buffer). This drives the emulator's LPUART
/// **RX** — the first real-firmware validation of serial *input* — and observes
/// the GPIO response by SwiftIO id. Ignored by default.
#[test]
#[ignore = "runs ~70M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_uart_rx_switches_the_led() {
    const UART: &[u8] = include_bytes!("fixtures/madmachine_swiftio_uart.elf");
    let image = loader::load_elf(UART).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    // Boot to the main polling loop.
    for _ in 0..40_000_000u64 {
        soc.step();
    }
    assert_eq!(soc.swiftio_pin(18), Some(false), "LED D18 starts off");

    // Send "1" over UART0 (LPUART2 = index 1) → LED on.
    soc.bus.periph.lpuart[1].rx_push(b'1');
    for _ in 0..20_000_000u64 {
        soc.step();
        if soc.swiftio_pin(18) == Some(true) {
            break;
        }
    }
    assert_eq!(soc.swiftio_pin(18), Some(true), "\"1\" turns the LED on");

    // Send "0" → LED off.
    soc.bus.periph.lpuart[1].rx_push(b'0');
    for _ in 0..20_000_000u64 {
        soc.step();
        if soc.swiftio_pin(18) == Some(false) {
            break;
        }
    }
    assert_eq!(soc.swiftio_pin(18), Some(false), "\"0\" turns the LED off");
}

/// The **real MadMachine LCD example** (`08LCD`): the MadDrivers `ST7789`
/// driver over `SPI(Id.SPI0)` filling a 240×240 display with solid colours.
/// SwiftIO `Id.SPI0` is LPSPI3; the driver is interrupt-driven
/// (`LPSPI_MasterTransferNonBlocking` + `spi_context_wait`, IRQ-34 ISR). This
/// records the MOSI bytes from an attached display: `clearScreen(red)`
/// (`0xF800` → high byte `0xF8`) writes 57 600 red pixels, so seeing tens of
/// thousands of `0xF8` bytes proves the ST7789 renders over SPI. Ignored by
/// default (~200M instructions, including the ST7789 ~120 ms reset delay).
#[test]
#[ignore = "runs ~200M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_lcd_fills_the_screen_over_spi() {
    use std::sync::{Arc, Mutex};
    struct Recorder {
        hist: Arc<Mutex<[u64; 256]>>,
    }
    impl rt1060_rs::peripherals::lpspi::SpiDevice for Recorder {
        fn transfer(&mut self, mosi: u32) -> u32 {
            self.hist.lock().unwrap()[(mosi & 0xff) as usize] += 1;
            0
        }
    }

    const LCD: &[u8] = include_bytes!("fixtures/madmachine_swiftio_lcd.elf");
    let image = loader::load_elf(LCD).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    let hist = Arc::new(Mutex::new([0u64; 256]));
    // SwiftIO Id.SPI0 = LPSPI3 = index 2.
    soc.bus.periph.lpspi[2].attach(Box::new(Recorder { hist: hist.clone() }));

    let red = |h: &Arc<Mutex<[u64; 256]>>| h.lock().unwrap()[0xF8];
    let mut steps = 0u64;
    while steps < 300_000_000 && red(&hist) < 50_000 {
        for _ in 0..2_000_000 {
            soc.step();
        }
        steps += 2_000_000;
    }
    let seen = red(&hist);
    assert!(
        seen >= 50_000,
        "the ST7789 driver should fill the screen red over SPI (saw {seen} red bytes)"
    );
}

/// A minimal Sensirion **SHT3x** humidity/temperature sensor on I2C (address
/// `0x44`). It answers any command by returning the fixed 6-byte measurement
/// `[tempMSB, tempLSB, crc, humMSB, humLSB, crc]`; the MadDrivers `SHT3x` driver
/// ignores the CRC bytes. `temp = 175*raw/65535 − 45`, `humidity = 100*raw/65535`.
struct Sht3x {
    data: [u8; 6],
    idx: usize,
}

impl Sht3x {
    fn new(temp_c: f32, humidity: f32) -> Self {
        let raw_t = ((temp_c + 45.0) / 175.0 * 65535.0) as u16;
        let raw_h = (humidity / 100.0 * 65535.0) as u16;
        Self {
            data: [
                (raw_t >> 8) as u8,
                raw_t as u8,
                0,
                (raw_h >> 8) as u8,
                raw_h as u8,
                0,
            ],
            idx: 0,
        }
    }
}

impl rt1060_rs::peripherals::lpi2c::I2cDevice for Sht3x {
    fn address(&self) -> u8 {
        0x44
    }
    fn start(&mut self, read: bool) -> bool {
        if read {
            self.idx = 0; // a read transaction restarts the response
        }
        true
    }
    fn write(&mut self, _byte: u8) -> bool {
        true // accept every command byte (soft reset, measure, …)
    }
    fn read(&mut self) -> u8 {
        let b = self.data.get(self.idx).copied().unwrap_or(0);
        self.idx += 1;
        b
    }
}

/// The **real MadMachine Humiture example** (`05Humiture`): the MadDrivers
/// `SHT3x` driver over `I2C(Id.I2C0)` reading temperature + humidity and
/// printing them. SwiftIO `Id.I2C0` is LPI2C3; the driver is interrupt-driven
/// (`LPI2C_MasterTransferNonBlocking` + `k_sem_take`, completed by the IRQ-30
/// state-machine ISR), so this exercises the whole LPI2C interrupt path plus a
/// real sensor driver. With an emulated SHT3x on the bus the firmware performs
/// the real measure-then-read transaction and prints the decoded values.
/// Ignored by default.
#[test]
#[ignore = "runs ~40M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_humiture_reads_the_i2c_sensor() {
    const I2C: &[u8] = include_bytes!("fixtures/madmachine_swiftio_i2c.elf");
    let image = loader::load_elf(I2C).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    // SwiftIO Id.I2C0 = LPI2C3 = index 2.
    soc.bus.periph.lpi2c[2].attach(Box::new(Sht3x::new(25.0, 50.0)));
    let mut console = String::new();
    for _ in 0..60_000_000u64 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!("unimplemented instruction {hw:#06x} in Humiture boot");
        }
        console.push_str(&soc.console_string());
        if console.contains("Temperature: 25.0") {
            break;
        }
    }
    assert!(
        console.contains("Temperature: 25.0"),
        "the firmware should read and print the sensor temperature: {console:?}"
    );
}

/// The **real MadMachine BreathingLED example** (`03Buzzer/BreathingLED`):
/// `PWMOut(Id.PWM4A).setDutycycle(d)` with `d` ramping 0→1→0. It drives the
/// FlexPWM peripheral, and the emulator observes the duty via `pwm_duty` —
/// proof a real Swift program controls PWM output. Ignored by default.
#[test]
#[ignore = "runs ~150M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_breathing_led_ramps_the_pwm_duty() {
    use rt1060_rs::peripherals::pwm::Chan;
    const PWM: &[u8] = include_bytes!("fixtures/madmachine_swiftio_pwm.elf");
    let image = loader::load_elf(PWM).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();

    // PWM4A resolves to FlexPWM4 submodule 3, channel A.
    let duty = |soc: &Rt1060| soc.pwm_duty(4, 3, Chan::A);
    let mut early = None;
    let mut steps = 0u64;
    while steps < 200_000_000 {
        soc.step();
        steps += 1;
        // Capture the duty once the ramp is under way, then again well later.
        if early.is_none() && duty(&soc).is_some_and(|d| d > 0.05) {
            early = Some(duty(&soc).unwrap());
            break;
        }
    }
    let early = early.expect("the LED PWM duty should become observable");
    for _ in 0..60_000_000u64 {
        soc.step();
    }
    let later = duty(&soc).expect("PWM still driven");
    assert!(
        later > early,
        "the breathing LED should ramp the PWM duty up ({early} -> {later})"
    );
}

/// The **real MadMachine Potentiometer example** (`AnalogIn(A0).readVoltage()`
/// printed once a second), built with the MadMachine SDK. It exercises the
/// full external-interrupt path — the ADC's conversion-complete IRQ routes
/// through Zephyr's `_isr_wrapper`, which reads `IPSR` (the bug that broke
/// every external IRQ) — plus the ADC model and Swift's `print`. Driving every
/// ADC input to 3/4 scale makes the firmware print ~2.47 V (of the 3.3 V ref).
/// Ignored by default (tens of millions of instructions through Zephyr).
#[test]
#[ignore = "runs ~30M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_potentiometer_reads_the_adc() {
    const POT: &[u8] = include_bytes!("fixtures/madmachine_swiftio_pot.elf");
    let image = loader::load_elf(POT).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    // 0xC00 = 3072 of 4095 → 3072/4095 * 3.3 V ≈ 2.47 V.
    for adc in soc.bus.periph.adc.iter_mut() {
        for ch in 0..16 {
            adc.set_channel(ch, 3072);
        }
    }
    for _ in 0..30_000_000u64 {
        soc.step();
        if let Some(BreakCause::Unimplemented(hw)) = soc.core.break_cause {
            panic!("unimplemented instruction {hw:#06x} in Potentiometer boot");
        }
    }
    let console = soc.console_string();
    assert!(
        console.contains("2.47"),
        "the firmware should read and print the driven ADC voltage (~2.47 V): {console:?}"
    );
}

/// Deep check: attach an SD card and confirm the real MadMachine Zephyr SD
/// driver runs its full init — controller reset, the CMD0/8/ACMD41/CMD2/CMD3/
/// CMD9/CMD7 identification, ACMD51 SCR, and the CMD6 timing / driver-strength
/// / current-limit switches — and reaches the block-read data phase (the
/// card-init read of sector 0). A blank card is enough: reaching the read at
/// all proves the SYS_CTRL reset strobes self-cleared and the CMD6 switches
/// were accepted. Ignored by default (tens of millions of instructions);
/// run with `cargo test --release -- --ignored`.
#[test]
#[ignore = "runs ~40M instructions through Zephyr; cargo test --release -- --ignored"]
fn madmachine_initializes_an_attached_sd_card() {
    use rt1060_rs::peripherals::usdhc::SdCard;
    const BLINK: &[u8] = include_bytes!("fixtures/madmachine_swiftio_blink.elf");
    let image = loader::load_elf(BLINK).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();
    soc.insert_sd_card(1, SdCard::blank(2048)); // a 1 MiB card, no filesystem

    for _ in 0..45_000_000u64 {
        soc.step();
        if soc.bus.periph.usdhc[0].card_reads() > 0 {
            break;
        }
    }
    assert!(
        soc.bus.periph.usdhc[0].card_reads() > 0,
        "the SD driver should complete init and read a block from the card"
    );
}

/// Deep check: run long enough for the delay loop to elapse and observe the
/// LED actually toggle. Ignored by default (hundreds of millions of
/// instructions); run with `cargo test --release -- --ignored`.
#[test]
#[ignore = "runs ~250M instructions; cargo test --release -- --ignored"]
fn sdram_blinky_toggles_the_led() {
    let image = loader::load_elf(SDRAM).expect("parse ELF");
    let mut soc = Rt1060::boot(&image);
    soc.quiet();

    let led = |soc: &Rt1060| soc.bus.periph.gpio[0].output(9);
    let mut transitions = 0;
    let mut last = led(&soc);
    for _ in 0..250_000_000u64 {
        soc.step();
        let now = led(&soc);
        if now != last {
            transitions += 1;
            last = now;
            if transitions >= 2 {
                break;
            }
        }
    }
    assert!(
        transitions >= 2,
        "the LED should have blinked at least twice (saw {transitions})"
    );
}

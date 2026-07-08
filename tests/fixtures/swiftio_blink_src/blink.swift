// Bare-metal embedded-Swift blink for the NXP RT1062 (SwiftIO Micro).
// Drives the onboard RGB LED via GPIO1 — pins recovered from the HalSwiftIO
// gpio_pin_maps table: RED = GPIO1 pin 9, BLUE = GPIO1 pin 11.

let GPIO1: UInt = 0x401B_8000
let GDIR = UnsafeMutablePointer<UInt32>(bitPattern: GPIO1 + 0x04)!
let DR_SET = UnsafeMutablePointer<UInt32>(bitPattern: GPIO1 + 0x84)!
let DR_CLEAR = UnsafeMutablePointer<UInt32>(bitPattern: GPIO1 + 0x88)!

let RED_BLUE: UInt32 = (1 << 9) | (1 << 11)

@_cdecl("Reset_Handler")
func resetHandler() {
    GDIR.pointee = RED_BLUE            // configure RED/BLUE as outputs
    var on = false
    while true {
        if on { DR_SET.pointee = RED_BLUE } else { DR_CLEAR.pointee = RED_BLUE }
        on = !on
        var i: UInt32 = 0
        while i < 4000 { i &+= 1 }     // delay
    }
}

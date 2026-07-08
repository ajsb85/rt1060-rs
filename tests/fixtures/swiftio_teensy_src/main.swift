import SwiftIO
import MadBoard

// D16 = GPIO2 IO3 = pad GPIO_B0_03 = the Teensy 4.1 onboard LED (pin 13).
let led = DigitalOut(Id.D16)

while true {
    led.toggle()
    sleep(ms: 500)
}

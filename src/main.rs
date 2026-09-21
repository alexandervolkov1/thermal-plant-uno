#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use arduino_hal::prelude::*;
use core::cell::Cell;
use panic_halt as _;

static MILLIS: avr_device::interrupt::Mutex<Cell<u32>> =
    avr_device::interrupt::Mutex::new(Cell::new(0));

fn millis_init(timer: arduino_hal::pac::TC0) {
    timer.tccr0a().write(|w| w.wgm0().ctc());

    timer.ocr0a().write(|w| w.set(249));

    timer.tccr0b().write(|w| w.cs0().prescale_64());

    timer.timsk0().write(|w| w.ocie0a().set_bit());
}

#[avr_device::interrupt(atmega328p)]
fn TIMER0_COMPA() {
    avr_device::interrupt::free(|cs| {
        let millis = MILLIS.borrow(cs);

        millis.set(millis.get().wrapping_add(1));
    });
}

fn millis() -> u32 {
    avr_device::interrupt::free(|cs| MILLIS.borrow(cs).get())
}

#[arduino_hal::entry]
fn main() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    let mut serial = arduino_hal::default_serial!(dp, pins, 57600);
    let mut led = pins.d13.into_output();

    millis_init(dp.TC0);

    unsafe {
        avr_device::interrupt::enable();
    }

    let mut last_rx_time = millis();

    let mut led_is_on = false;

    loop {
        match serial.read() {
            Ok(value) => {
                led.set_high();
                led_is_on = true;
                last_rx_time = millis();
                nb::block!(serial.write(value)).unwrap_infallible();
            }
            Err(nb::Error::WouldBlock) => {}
            Err(nb::Error::Other(_)) => panic!(),
        }

        let now = millis();

        if led_is_on && now.wrapping_sub(last_rx_time) >= 50 {
            led.set_low();
            led_is_on = false;
        }
    }
}

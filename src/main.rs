#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

mod model;

use arduino_hal::prelude::*;
use core::cell::Cell;
use model::{ThermalInput, ThermalParams, ThermalState, rk2_step};
use panic_halt as _;

const MODEL_PERIOD_MS: u32 = 100;
const MODEL_DT: f32 = 0.1;

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

fn parse_percent(bytes: &[u8]) -> Option<u8> {
    match bytes {
        [single @ b'0'..=b'9'] => Some(single - b'0'),
        [first @ b'1'..=b'9', second @ b'0'..=b'9'] => Some((first - b'0') * 10 + (second - b'0')),
        [b'1', b'0', b'0'] => Some(100),
        _ => None,
    }
}

#[arduino_hal::entry]
fn main() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    let mut serial = arduino_hal::default_serial!(dp, pins, 57600);

    millis_init(dp.TC0);

    unsafe {
        avr_device::interrupt::enable();
    }

    let mut state = ThermalState {
        heater_temp: 20.0,
        sample_temp: 20.0,
    };

    let params = ThermalParams {
        heater_capacity: 100.0,
        sample_capacity: 200.0,

        coupling: 2.0,

        heater_loss: 1.0,
        sample_loss: 0.5,

        max_heater_power: 100.0,
    };

    let mut input = ThermalInput {
        ambient_temp: 20.0,
        heater_command: 0.0,
    };

    let mut last_model_step = millis();

    let mut command_buffer = [0u8; 16];
    let mut command_len: usize = 0;

    let mut discarding_command = false;

    loop {
        match serial.read() {
            Ok(value) => {
                if value == b'\r' || value == b'\n' {
                    if discarding_command {
                        ufmt::uwriteln!(&mut serial, "ERR").unwrap_infallible();
                        discarding_command = false;
                        command_len = 0;
                    } else if command_len > 0 {
                        let command = &command_buffer[..command_len];

                        match command {
                            [b't'] => {
                                let scaled = libm::roundf(state.sample_temp * 100.0) as i32;

                                let whole = scaled / 100;
                                let fraction = scaled.abs() % 100;

                                if fraction < 10 {
                                    ufmt::uwriteln!(&mut serial, "TEMP {}.0{}", whole, fraction)
                                        .unwrap_infallible();
                                } else {
                                    ufmt::uwriteln!(&mut serial, "TEMP {}.{}", whole, fraction)
                                        .unwrap_infallible();
                                }
                            }

                            [b'p', percent @ ..] => {
                                if let Some(percent) = parse_percent(percent) {
                                    input.heater_command = percent as f32 / 100.0;
                                    ufmt::uwriteln!(&mut serial, "OK").unwrap_infallible();
                                } else {
                                    ufmt::uwriteln!(&mut serial, "ERR").unwrap_infallible();
                                }
                            }

                            _ => {
                                ufmt::uwriteln!(&mut serial, "ERR").unwrap_infallible();
                            }
                        }

                        command_len = 0;
                    }
                } else if discarding_command {
                } else if command_len < command_buffer.len() {
                    command_buffer[command_len] = value;
                    command_len += 1;
                } else {
                    command_len = 0;
                    discarding_command = true;
                }
            }
            Err(nb::Error::WouldBlock) => {}
            Err(nb::Error::Other(_)) => panic!(),
        }

        let now = millis();

        if now.wrapping_sub(last_model_step) >= MODEL_PERIOD_MS {
            state = rk2_step(state, params, input, MODEL_DT);

            last_model_step = last_model_step.wrapping_add(MODEL_PERIOD_MS);
        }
    }
}

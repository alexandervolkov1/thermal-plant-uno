#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

mod model;
mod protocol;

use arduino_hal::prelude::*;
use core::cell::Cell;
use model::{ThermalInput, ThermalParams, ThermalState, rk2_step};
use panic_halt as _;

use crate::protocol::{Command, parse_command};

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

fn write_temperature<W>(writer: &mut W, label: &str, value: f32) -> Result<(), W::Error>
where
    W: ufmt::uWrite + ?Sized,
{
    let scaled = libm::roundf(value * 100.0) as i32;

    let negative = scaled < 0;
    let magnitude = scaled.unsigned_abs();

    let whole = magnitude / 100;
    let fraction = magnitude % 100;

    ufmt::uwrite!(writer, "{} ", label)?;

    if negative {
        ufmt::uwrite!(writer, "-")?;
    }

    if fraction < 10 {
        ufmt::uwriteln!(writer, "{}.0{}", whole, fraction)
    } else {
        ufmt::uwriteln!(writer, "{}.{}", whole, fraction)
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
                        if let Some(command) = parse_command(&command_buffer[..command_len]) {
                            match command {
                                Command::Identity => {
                                    ufmt::uwriteln!(&mut serial, "ID THERMAL_PLANT 1")
                                        .unwrap_infallible();
                                }

                                Command::SampleTemperature => {
                                    write_temperature(
                                        &mut serial,
                                        "SAMPLE_TEMP",
                                        state.sample_temp,
                                    )
                                    .unwrap_infallible();
                                }

                                Command::HeaterTemperature => {
                                    write_temperature(
                                        &mut serial,
                                        "HEATER_TEMP",
                                        state.heater_temp,
                                    )
                                    .unwrap_infallible();
                                }

                                Command::AmbientTemperature => {
                                    write_temperature(
                                        &mut serial,
                                        "AMBIENT_TEMP",
                                        input.ambient_temp,
                                    )
                                    .unwrap_infallible();
                                }

                                Command::GetPower => {
                                    let percent = libm::roundf(input.heater_command * 100.0) as u8;
                                    ufmt::uwriteln!(&mut serial, "POWER {}", percent)
                                        .unwrap_infallible();
                                }

                                Command::SetPower(percent) => {
                                    input.heater_command = percent as f32 / 100.0;
                                    ufmt::uwriteln!(&mut serial, "OK").unwrap_infallible();
                                }
                            }
                        } else {
                            ufmt::uwriteln!(&mut serial, "ERR").unwrap_infallible();
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

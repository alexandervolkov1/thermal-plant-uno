#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

mod protocol;

use arduino_hal::prelude::*;
use core::cell::Cell;
use panic_halt as _;
use thermal_plant_model::{SimulationConfig, ThermalPlant};

use crate::protocol::{Action, Command, Scope, parse, parse_command, parse_percent};

const MODEL_PERIOD_MS: u32 = 100;

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
    ufmt::uwrite!(writer, "{} ", label)?;

    write_fixed_2(writer, value)?;

    ufmt::uwriteln!(writer, "")
}

fn write_fixed_2<W>(writer: &mut W, value: f32) -> Result<(), W::Error>
where
    W: ufmt::uWrite + ?Sized,
{
    let scaled = libm::roundf(value * 100.0) as i32;

    let negative = scaled < 0;
    let magnitude = scaled.unsigned_abs();

    let whole = magnitude / 100;
    let fraction = magnitude % 100;

    if negative {
        ufmt::uwrite!(writer, "-")?;
    }

    if fraction < 10 {
        ufmt::uwrite!(writer, "{}.0{}", whole, fraction)
    } else {
        ufmt::uwrite!(writer, "{}.{}", whole, fraction)
    }
}

fn write_reply_temperature<W>(writer: &mut W, id: u16, value: f32) -> Result<(), W::Error>
where
    W: ufmt::uWrite + ?Sized,
{
    ufmt::uwrite!(writer, "@{} ok ", id)?;
    write_fixed_2(writer, value)?;
    ufmt::uwriteln!(writer, "")
}

fn handle_operator_request<W>(
    writer: &mut W,
    id: u16,
    action: Action<'_>,
    plant: &mut ThermalPlant,
) -> Result<(), W::Error>
where
    W: ufmt::uWrite + ?Sized,
{
    match action {
        Action::Get(key) if key == b"t.sample" => {
            write_reply_temperature(writer, id, plant.measured_temperatures().sample_temp)
        }

        Action::Get(key) if key == b"t.heater" => {
            write_reply_temperature(writer, id, plant.measured_temperatures().heater_temp)
        }

        Action::Get(key) if key == b"t.room" => {
            write_reply_temperature(writer, id, plant.measured_temperatures().room_temp)
        }

        Action::Get(key) if key == b"power" => {
            let percent = libm::roundf(plant.power_percent()) as u8;
            ufmt::uwriteln!(writer, "@{} ok {}", id, percent)
        }

        Action::Set { key, value } if key == b"power" => match parse_percent(value) {
            Some(percent) => {
                if plant.set_power_percent(percent as f32).is_ok() {
                    ufmt::uwriteln!(writer, "@{} ok", id)
                } else {
                    ufmt::uwriteln!(writer, "@{} err value", id)
                }
            }
            None => ufmt::uwriteln!(writer, "@{} err value", id),
        },

        Action::Get(_) | Action::Set { .. } => {
            ufmt::uwriteln!(writer, "@{} err key", id)
        }

        Action::Apply | Action::Reset => {
            ufmt::uwriteln!(writer, "@{} err unsupported", id)
        }
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

    let config = SimulationConfig::default();
    let mut plant = ThermalPlant::new(config).unwrap();

    let mut last_model_step = millis();

    let mut command_buffer = [0u8; 64];
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
                                        plant.measured_temperatures().sample_temp,
                                    )
                                    .unwrap_infallible();
                                }

                                Command::HeaterTemperature => {
                                    write_temperature(
                                        &mut serial,
                                        "HEATER_TEMP",
                                        plant.measured_temperatures().heater_temp,
                                    )
                                    .unwrap_infallible();
                                }

                                Command::RoomTemperature => {
                                    write_temperature(
                                        &mut serial,
                                        "ROOM_TEMP",
                                        plant.measured_temperatures().room_temp,
                                    )
                                    .unwrap_infallible();
                                }

                                Command::AmbientTemperature => {
                                    write_temperature(
                                        &mut serial,
                                        "AMBIENT_TEMP",
                                        config.background_temp,
                                    )
                                    .unwrap_infallible();
                                }

                                Command::GetPower => {
                                    let percent = libm::roundf(plant.power_percent()) as u8;
                                    ufmt::uwriteln!(&mut serial, "POWER {}", percent)
                                        .unwrap_infallible();
                                }

                                Command::SetPower(percent) => {
                                    plant.set_power_percent(percent as f32).unwrap();
                                    ufmt::uwriteln!(&mut serial, "OK").unwrap_infallible();
                                }

                                Command::Reset => {
                                    plant = ThermalPlant::new(config).unwrap();

                                    last_model_step = millis();

                                    ufmt::uwriteln!(&mut serial, "OK").unwrap_infallible();
                                }

                                Command::State => {
                                    let state = plant.measured_temperatures();

                                    ufmt::uwrite!(&mut serial, "STATE ").unwrap_infallible();
                                    write_fixed_2(&mut serial, state.heater_temp)
                                        .unwrap_infallible();

                                    ufmt::uwrite!(&mut serial, " ").unwrap_infallible();
                                    write_fixed_2(&mut serial, state.sample_temp)
                                        .unwrap_infallible();

                                    ufmt::uwrite!(&mut serial, " ").unwrap_infallible();
                                    write_fixed_2(&mut serial, config.background_temp)
                                        .unwrap_infallible();

                                    let percent = libm::roundf(plant.power_percent()) as u8;

                                    ufmt::uwriteln!(&mut serial, " {}", percent)
                                        .unwrap_infallible();
                                }
                            }
                        } else if command_buffer[0] == b'o' || command_buffer[0] == b'd' {
                            match parse(&command_buffer[..command_len]) {
                                Ok(request) => match request.scope {
                                    Scope::Operator => {
                                        handle_operator_request(
                                            &mut serial,
                                            request.id,
                                            request.action,
                                            &mut plant,
                                        )
                                        .unwrap_infallible();
                                    }

                                    Scope::Designer => {
                                        ufmt::uwriteln!(
                                            &mut serial,
                                            "@{} err unsupported",
                                            request.id
                                        )
                                        .unwrap_infallible();
                                    }
                                },

                                Err(error) => {
                                    if let Some(id) = error.id {
                                        ufmt::uwriteln!(&mut serial, "@{} err syntax", id)
                                            .unwrap_infallible();
                                    } else {
                                        ufmt::uwriteln!(&mut serial, "ERR").unwrap_infallible();
                                    }
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
            plant.step();

            last_model_step = last_model_step.wrapping_add(MODEL_PERIOD_MS);
        }
    }
}

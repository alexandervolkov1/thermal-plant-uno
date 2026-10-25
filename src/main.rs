#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), feature(abi_avr_interrupt))]

mod decimal;
mod protocol;

#[cfg(test)]
#[path = "../tests/operator.rs"]
mod tests;

#[cfg(not(test))]
use arduino_hal::prelude::*;
#[cfg(not(test))]
use core::cell::Cell;
#[cfg(not(test))]
use panic_halt as _;
use thermal_plant_model::{SimulationConfig, ThermalPlant};

use crate::protocol::{
    Action, Command, Scope, parse, parse_command, parse_percent, parse_u32, token_eq,
};

#[cfg(not(test))]
const MODEL_PERIOD_MS: u32 = 100;

#[cfg(not(test))]
static MILLIS: avr_device::interrupt::Mutex<Cell<u32>> =
    avr_device::interrupt::Mutex::new(Cell::new(0));

#[cfg(not(test))]
fn millis_init(timer: arduino_hal::pac::TC0) {
    timer.tccr0a().write(|w| w.wgm0().ctc());

    timer.ocr0a().write(|w| w.set(249));

    timer.tccr0b().write(|w| w.cs0().prescale_64());

    timer.timsk0().write(|w| w.ocie0a().set_bit());
}

#[cfg(not(test))]
#[avr_device::interrupt(atmega328p)]
fn TIMER0_COMPA() {
    avr_device::interrupt::free(|cs| {
        let millis = MILLIS.borrow(cs);

        millis.set(millis.get().wrapping_add(1));
    });
}

#[cfg(not(test))]
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
        Action::Get(key) if token_eq(key, b"t.sample") => {
            write_reply_temperature(writer, id, plant.measured_temperatures().sample_temp)
        }

        Action::Get(key) if token_eq(key, b"t.heater") => {
            write_reply_temperature(writer, id, plant.measured_temperatures().heater_temp)
        }

        Action::Get(key) if token_eq(key, b"t.room") => {
            write_reply_temperature(writer, id, plant.measured_temperatures().room_temp)
        }

        Action::Get(key) if token_eq(key, b"power") => {
            let percent = libm::roundf(plant.power_percent()) as u8;
            ufmt::uwriteln!(writer, "@{} ok {}", id, percent)
        }

        Action::Get(key) if token_eq(key, b"device") => {
            ufmt::uwriteln!(writer, "@{} ok THERMAL_PLANT 2", id)
        }

        Action::Set { key, value } if token_eq(key, b"power") => match parse_percent(value) {
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

fn designer_parameter<'a>(config: &'a mut SimulationConfig, key: &[u8]) -> Option<&'a mut f32> {
    if token_eq(key, b"heater.capacity") {
        Some(&mut config.parameters.heater_capacity)
    } else if token_eq(key, b"sample.capacity") {
        Some(&mut config.parameters.sample_capacity)
    } else if token_eq(key, b"room.capacity") {
        Some(&mut config.parameters.room_capacity)
    } else if token_eq(key, b"coupling") {
        Some(&mut config.parameters.coupling)
    } else if token_eq(key, b"heater.loss") {
        Some(&mut config.parameters.heater_loss)
    } else if token_eq(key, b"sample.loss") {
        Some(&mut config.parameters.sample_loss)
    } else if token_eq(key, b"room.loss") {
        Some(&mut config.parameters.room_loss)
    } else if token_eq(key, b"heater.radiation") {
        Some(&mut config.parameters.heater_radiation)
    } else if token_eq(key, b"sample.radiation") {
        Some(&mut config.parameters.sample_radiation)
    } else if token_eq(key, b"heater.max_power") {
        Some(&mut config.parameters.max_heater_power)
    } else if token_eq(key, b"initial.heater") {
        Some(&mut config.initial_state.heater_temp)
    } else if token_eq(key, b"initial.sample") {
        Some(&mut config.initial_state.sample_temp)
    } else if token_eq(key, b"initial.room") {
        Some(&mut config.initial_state.room_temp)
    } else if token_eq(key, b"background.temp") {
        Some(&mut config.background_temp)
    } else if token_eq(key, b"parameter.spread") {
        Some(&mut config.parameter_spread)
    } else if token_eq(key, b"power.fluctuation") {
        Some(&mut config.heater_power_fluctuation)
    } else if token_eq(key, b"sensor.noise") {
        Some(&mut config.sensor_noise)
    } else if token_eq(key, b"sensor.drift") {
        Some(&mut config.sensor_drift_amplitude)
    } else {
        None
    }
}

fn handle_designer_request<W>(
    writer: &mut W,
    id: u16,
    action: Action<'_>,
    plant: &mut ThermalPlant,
    active_config: &mut SimulationConfig,
    staged_config: &mut SimulationConfig,
) -> Result<bool, W::Error>
where
    W: ufmt::uWrite + ?Sized,
{
    match action {
        Action::Get(key) if token_eq(key, b"seed") => {
            ufmt::uwriteln!(writer, "@{} ok {}", id, staged_config.seed)?;
        }
        Action::Get(key) => {
            if let Some(value) = designer_parameter(staged_config, key) {
                ufmt::uwrite!(writer, "@{} ok ", id)?;
                decimal::write_decimal(writer, *value)?;
                ufmt::uwriteln!(writer, "")?;
            } else {
                ufmt::uwriteln!(writer, "@{} err key", id)?;
            }
        }
        Action::Set { key, value } if token_eq(key, b"seed") => {
            if let Some(value) = parse_u32(value) {
                staged_config.seed = value;
                ufmt::uwriteln!(writer, "@{} ok", id)?;
            } else {
                ufmt::uwriteln!(writer, "@{} err value", id)?;
            }
        }
        Action::Set { key, value } => {
            if let Some(parameter) = designer_parameter(staged_config, key) {
                if let Some(value) = decimal::parse_decimal(value, token_eq(key, b"room.capacity"))
                {
                    *parameter = value;
                    ufmt::uwriteln!(writer, "@{} ok", id)?;
                } else {
                    ufmt::uwriteln!(writer, "@{} err value", id)?;
                }
            } else {
                ufmt::uwriteln!(writer, "@{} err key", id)?;
            }
        }
        Action::Apply => match ThermalPlant::new(*staged_config) {
            Ok(replacement) => {
                *plant = replacement;
                *active_config = *staged_config;
                ufmt::uwriteln!(writer, "@{} ok", id)?;
                return Ok(true);
            }
            Err(_) => ufmt::uwriteln!(writer, "@{} err config", id)?,
        },
        Action::Reset => match ThermalPlant::new(*active_config) {
            Ok(replacement) => {
                *plant = replacement;
                *staged_config = *active_config;
                ufmt::uwriteln!(writer, "@{} ok", id)?;
                return Ok(true);
            }
            Err(_) => ufmt::uwriteln!(writer, "@{} err config", id)?,
        },
    }
    Ok(false)
}

fn write_syntax_error<W>(writer: &mut W, bytes: &[u8]) -> Result<(), W::Error>
where
    W: ufmt::uWrite + ?Sized,
{
    let id = match parse(bytes) {
        Ok(request) => Some(request.id),
        Err(error) => error.id,
    };
    match id {
        Some(id) => ufmt::uwriteln!(writer, "@{} err syntax", id),
        None => ufmt::uwriteln!(writer, "ERR"),
    }
}

fn handle_scoped_request<W>(
    writer: &mut W,
    bytes: &[u8],
    plant: &mut ThermalPlant,
    active_config: &mut SimulationConfig,
    staged_config: &mut SimulationConfig,
) -> Result<bool, W::Error>
where
    W: ufmt::uWrite + ?Sized,
{
    match parse(bytes) {
        Ok(request) => match request.scope {
            Scope::Operator => {
                handle_operator_request(writer, request.id, request.action, plant)?;
                Ok(false)
            }
            Scope::Designer => handle_designer_request(
                writer,
                request.id,
                request.action,
                plant,
                active_config,
                staged_config,
            ),
        },
        Err(_) => {
            write_syntax_error(writer, bytes)?;
            Ok(false)
        }
    }
}

#[cfg(not(test))]
#[arduino_hal::entry]
fn main() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    let mut serial = arduino_hal::default_serial!(dp, pins, 57600);

    millis_init(dp.TC0);

    unsafe {
        avr_device::interrupt::enable();
    }

    let mut active_config = SimulationConfig::default();
    let mut staged_config = active_config;
    let mut plant = ThermalPlant::new(active_config).unwrap();

    let mut last_model_step = millis();

    let mut command_buffer = [0u8; 64];
    let mut command_len: usize = 0;

    let mut discarding_command = false;

    loop {
        match serial.read() {
            Ok(value) => {
                if value == b'\r' || value == b'\n' {
                    if discarding_command {
                        write_syntax_error(&mut serial, &command_buffer[..command_len])
                            .unwrap_infallible();
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
                                        active_config.background_temp,
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
                                    plant = ThermalPlant::new(active_config).unwrap();

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
                                    write_fixed_2(&mut serial, active_config.background_temp)
                                        .unwrap_infallible();

                                    let percent = libm::roundf(plant.power_percent()) as u8;

                                    ufmt::uwriteln!(&mut serial, " {}", percent)
                                        .unwrap_infallible();
                                }
                            }
                        } else if command_buffer[0] == b'o' || command_buffer[0] == b'd' {
                            if handle_scoped_request(
                                &mut serial,
                                &command_buffer[..command_len],
                                &mut plant,
                                &mut active_config,
                                &mut staged_config,
                            )
                            .unwrap_infallible()
                            {
                                last_model_step = millis();
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

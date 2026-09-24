#[derive(Clone, Copy)]
pub struct ThermalState {
    pub heater_temp: f32,
    pub sample_temp: f32,
}

#[derive(Clone, Copy)]
pub struct ThermalParams {
    pub heater_capacity: f32,
    pub sample_capacity: f32,

    pub coupling: f32,

    pub heater_loss: f32,
    pub sample_loss: f32,

    pub max_heater_power: f32,
}

#[derive(Clone, Copy)]
pub struct ThermalInput {
    pub ambient_temp: f32,
    pub heater_command: f32,
}

#[derive(Clone, Copy)]
pub struct ThermalDerivative {
    pub heater_temp_rate: f32,
    pub sample_temp_rate: f32,
}

pub fn derivatives(
    state: ThermalState,
    params: ThermalParams,
    input: ThermalInput,
) -> ThermalDerivative {
    let heater_power = params.max_heater_power * input.heater_command;

    let heater_to_sample = params.coupling * (state.heater_temp - state.sample_temp);

    let heater_to_ambient = params.heater_loss * (state.heater_temp - input.ambient_temp);

    let sample_to_ambient = params.sample_loss * (state.sample_temp - input.ambient_temp);

    let heater_temp_rate =
        (heater_power - heater_to_sample - heater_to_ambient) / params.heater_capacity;

    let sample_temp_rate = (heater_to_sample - sample_to_ambient) / params.sample_capacity;

    ThermalDerivative {
        heater_temp_rate,
        sample_temp_rate,
    }
}

pub fn euler_step(
    state: ThermalState,
    params: ThermalParams,
    input: ThermalInput,
    dt: f32,
) -> ThermalState {
    let ThermalDerivative {
        heater_temp_rate,
        sample_temp_rate,
    } = derivatives(state, params, input);

    ThermalState {
        heater_temp: state.heater_temp + heater_temp_rate * dt,
        sample_temp: state.sample_temp + sample_temp_rate * dt,
    }
}

pub fn rk2_step(
    state: ThermalState,
    params: ThermalParams,
    input: ThermalInput,
    dt: f32,
) -> ThermalState {
    let k1 = derivatives(state, params, input);

    let mid_state = ThermalState {
        heater_temp: state.heater_temp + k1.heater_temp_rate * dt / 2.0,
        sample_temp: state.sample_temp + k1.sample_temp_rate * dt / 2.0,
    };

    let k2 = derivatives(mid_state, params, input);

    ThermalState {
        heater_temp: state.heater_temp + k2.heater_temp_rate * dt,
        sample_temp: state.sample_temp + k2.sample_temp_rate * dt,
    }
}

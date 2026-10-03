pub enum Command {
    Identity,
    SampleTemperature,
    HeaterTemperature,
    AmbientTemperature,
    GetPower,
    SetPower(u8),
    Reset,
    State,
}

fn parse_percent(bytes: &[u8]) -> Option<u8> {
    match bytes {
        [single @ b'0'..=b'9'] => Some(single - b'0'),
        [first @ b'1'..=b'9', second @ b'0'..=b'9'] => Some((first - b'0') * 10 + (second - b'0')),
        [b'1', b'0', b'0'] => Some(100),
        _ => None,
    }
}

pub fn parse_command(bytes: &[u8]) -> Option<Command> {
    match bytes {
        [b'i'] => Some(Command::Identity),

        [b't', b's'] => Some(Command::SampleTemperature),

        [b't', b'h'] => Some(Command::HeaterTemperature),

        [b't', b'a'] => Some(Command::AmbientTemperature),

        [b'p'] => Some(Command::GetPower),

        [b'p', percent @ ..] => parse_percent(percent).map(Command::SetPower),

        [b'r'] => Some(Command::Reset),

        [b's'] => Some(Command::State),

        _ => None,
    }
}

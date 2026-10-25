pub enum Command {
    Identity,
    SampleTemperature,
    HeaterTemperature,
    RoomTemperature,
    AmbientTemperature,
    GetPower,
    SetPower(u8),
    Reset,
    State,
}

pub(crate) fn parse_percent(bytes: &[u8]) -> Option<u8> {
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

        [b't', b'r'] => Some(Command::RoomTemperature),

        [b't', b'a'] => Some(Command::AmbientTemperature),

        [b'p'] => Some(Command::GetPower),

        [b'p', percent @ ..] => parse_percent(percent).map(Command::SetPower),

        [b'r'] => Some(Command::Reset),

        [b's'] => Some(Command::State),

        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Operator,
    Designer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action<'a> {
    Get(&'a [u8]),
    Set { key: &'a [u8], value: &'a [u8] },
    Apply,
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub id: u16,
    pub scope: Scope,
    pub action: Action<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub id: Option<u16>,
}

fn parse_id(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() || bytes[0] == b'0' {
        return None;
    }

    let mut id = 0u16;

    for &digit in bytes {
        if !digit.is_ascii_digit() {
            return None;
        }

        id = id.checked_mul(10)?.checked_add(u16::from(digit - b'0'))?;
    }
    Some(id)
}

pub fn parse(bytes: &[u8]) -> Result<Request<'_>, ParseError> {
    let mut fields = bytes.split(|byte| *byte == b' ');

    let scope = match fields.next() {
        Some(value) if value == b"o" => Scope::Operator,
        Some(value) if value == b"d" => Scope::Designer,
        _ => return Err(ParseError { id: None }),
    };

    let id = fields
        .next()
        .and_then(parse_id)
        .ok_or(ParseError { id: None })?;

    let syntax_error = ParseError { id: Some(id) };

    let verb = fields.next().ok_or(syntax_error)?;

    let action = if verb == b"get" {
        let key = fields.next().ok_or(syntax_error)?;

        if key.is_empty() || fields.next().is_some() {
            return Err(syntax_error);
        }

        Action::Get(key)
    } else if verb == b"set" {
        let key = fields.next().ok_or(syntax_error)?;
        let value = fields.next().ok_or(syntax_error)?;

        if key.is_empty() || value.is_empty() || fields.next().is_some() {
            return Err(syntax_error);
        }

        Action::Set { key, value }
    } else if verb == b"apply" && scope == Scope::Designer && fields.next().is_none() {
        Action::Apply
    } else if verb == b"reset" && scope == Scope::Designer && fields.next().is_none() {
        Action::Reset
    } else {
        return Err(syntax_error);
    };

    Ok(Request { id, scope, action })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_operator_requests() {
        assert_eq!(
            parse(b"o 17 get t.sample"),
            Ok(Request {
                id: 17,
                scope: Scope::Operator,
                action: Action::Get(b"t.sample"),
            })
        );

        assert_eq!(
            parse(b"o 19 set power 50"),
            Ok(Request {
                id: 19,
                scope: Scope::Operator,
                action: Action::Set {
                    key: b"power",
                    value: b"50",
                },
            })
        );
    }

    #[test]
    fn parses_designer_requests() {
        assert_eq!(
            parse(b"d 21 set room.capacity 350000"),
            Ok(Request {
                id: 21,
                scope: Scope::Designer,
                action: Action::Set {
                    key: b"room.capacity",
                    value: b"350000",
                },
            })
        );

        assert_eq!(
            parse(b"d 22 apply"),
            Ok(Request {
                id: 22,
                scope: Scope::Designer,
                action: Action::Apply,
            })
        );

        assert_eq!(
            parse(b"d 23 reset"),
            Ok(Request {
                id: 23,
                scope: Scope::Designer,
                action: Action::Reset,
            })
        );
    }

    #[test]
    fn rejects_malformed_requests() {
        assert_eq!(parse(b"o 17 reset"), Err(ParseError { id: Some(17) }));

        assert_eq!(parse(b"o 17 get"), Err(ParseError { id: Some(17) }));

        assert_eq!(parse(b"d 65536 reset"), Err(ParseError { id: None }));

        assert_eq!(
            parse(b"o 17 get sample_temp extra"),
            Err(ParseError { id: Some(17) })
        );

        assert_eq!(
            parse(b"o 17  get sample_temp"),
            Err(ParseError { id: Some(17) })
        );
    }

    #[test]
    fn legacy_protocol_still_parses() {
        assert!(matches!(
            parse_command(b"ts"),
            Some(Command::SampleTemperature)
        ));

        assert!(matches!(
            parse_command(b"tr"),
            Some(Command::RoomTemperature)
        ));

        assert!(matches!(parse_command(b"p50"), Some(Command::SetPower(50))));
    }
}

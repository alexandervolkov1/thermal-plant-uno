use super::*;

#[derive(Default)]
struct Output(std::string::String);

impl ufmt::uWrite for Output {
    type Error = core::convert::Infallible;

    fn write_str(&mut self, text: &str) -> Result<(), Self::Error> {
        self.0.push_str(text);
        Ok(())
    }
}

fn reply(plant: &mut ThermalPlant, bytes: &[u8]) -> std::string::String {
    let mut output = Output::default();
    handle_scoped_request(&mut output, bytes, plant).unwrap();
    output.0
}

#[test]
fn operator_identity_and_power() {
    let mut plant = ThermalPlant::new(SimulationConfig::default()).unwrap();
    assert_eq!(
        reply(&mut plant, b"o 22 get device"),
        "@22 ok THERMAL_PLANT 2\n"
    );
    assert_eq!(reply(&mut plant, b"o 65535 get power"), "@65535 ok 0\n");
    for percent in 0..=100 {
        let request = std::format!("o 21 set power {percent}");
        assert_eq!(reply(&mut plant, request.as_bytes()), "@21 ok\n");
        assert_eq!(
            reply(&mut plant, b"o 20 get power"),
            std::format!("@20 ok {percent}\n")
        );
    }
}

#[test]
fn malformed_power_preserves_previous_setting() {
    let mut plant = ThermalPlant::new(SimulationConfig::default()).unwrap();
    plant.set_power_percent(50.0).unwrap();
    for value in [
        "101", "255", "999999", "-1", "+1", "1.5", "NaN", "1x", "01", "00",
    ] {
        let request = std::format!("o 24 set power {value}");
        assert_eq!(reply(&mut plant, request.as_bytes()), "@24 err value\n");
        assert_eq!(plant.power_percent(), 50.0);
    }
    for request in [
        "o 25 set power",
        "o 25 set power ",
        "o 25 set power 50 extra",
    ] {
        assert_eq!(reply(&mut plant, request.as_bytes()), "@25 err syntax\n");
        assert_eq!(plant.power_percent(), 50.0);
    }
}

#[test]
fn temperatures_use_sampled_sensor_values() {
    let mut config = SimulationConfig::default();
    config.sensor_noise = 2.0;
    config.initial_state.sample_temp = -10.46;
    config.initial_state.heater_temp = 26.07;
    let mut plant = ThermalPlant::new(config).unwrap();
    plant.set_power_percent(100.0).unwrap();
    for _ in 0..11 {
        plant.step();
    }
    let measured = plant.measured_temperatures();
    for (request, id, value) in [
        (&b"o 17 get t.sample"[..], 17, measured.sample_temp),
        (&b"o 18 get t.heater"[..], 18, measured.heater_temp),
        (&b"o 19 get t.room"[..], 19, measured.room_temp),
    ] {
        let mut expected = Output::default();
        ufmt::uwrite!(&mut expected, "@{} ok ", id).unwrap();
        write_fixed_2(&mut expected, value).unwrap();
        expected.0.push('\n');
        assert_eq!(reply(&mut plant, request), expected.0);
    }
}

#[test]
fn errors_and_designer_requests_preserve_ids() {
    let mut plant = ThermalPlant::new(SimulationConfig::default()).unwrap();
    for request in ["o 23 get unknown", "o 23 set device 1"] {
        assert_eq!(reply(&mut plant, request.as_bytes()), "@23 err key\n");
    }
    for request in [
        "o 25 get",
        "o 25 reset",
        "o 25  get power",
        "d 25 apply extra",
    ] {
        assert_eq!(reply(&mut plant, request.as_bytes()), "@25 err syntax\n");
    }
    for request in [
        "o 0 get power",
        "o 01 get power",
        "o 65536 get power",
        "x 25 get power",
    ] {
        assert_eq!(reply(&mut plant, request.as_bytes()), "ERR\n");
    }
    for request in [
        "d 26 get seed",
        "d 26 set seed 1",
        "d 26 apply",
        "d 26 reset",
    ] {
        assert_eq!(
            reply(&mut plant, request.as_bytes()),
            "@26 err unsupported\n"
        );
    }
    assert_eq!(plant.power_percent(), 0.0);
}

#[test]
fn overlong_requests_report_syntax_without_applying_a_truncated_value() {
    let mut plant = ThermalPlant::new(SimulationConfig::default()).unwrap();
    plant.set_power_percent(50.0).unwrap();
    for prefix in [
        &b"o 27 set power 100"[..],
        b"o 27 get power extra",
        b"d 27 set seed 1",
    ] {
        let mut output = Output::default();
        write_syntax_error(&mut output, prefix).unwrap();
        assert_eq!(output.0, "@27 err syntax\n");
        assert_eq!(plant.power_percent(), 50.0);
    }
    let mut output = Output::default();
    write_syntax_error(&mut output, b"p100000000").unwrap();
    assert_eq!(output.0, "ERR\n");
}

#[test]
fn temperature_formatting_and_legacy_labels_are_unchanged() {
    for (value, expected) in [
        (20.46, "20.46"),
        (26.07, "26.07"),
        (20.0, "20.00"),
        (-1.25, "-1.25"),
    ] {
        let mut output = Output::default();
        write_fixed_2(&mut output, value).unwrap();
        assert_eq!(output.0, expected);
    }
    let mut output = Output::default();
    write_temperature(&mut output, "SAMPLE_TEMP", 20.46).unwrap();
    assert_eq!(output.0, "SAMPLE_TEMP 20.46\n");
}

#[test]
fn legacy_commands_and_power_validation_are_unchanged() {
    for (bytes, matches_command) in [
        (
            &b"i"[..],
            matches!(parse_command(b"i"), Some(Command::Identity)),
        ),
        (
            b"th",
            matches!(parse_command(b"th"), Some(Command::HeaterTemperature)),
        ),
        (
            b"ta",
            matches!(parse_command(b"ta"), Some(Command::AmbientTemperature)),
        ),
        (b"p", matches!(parse_command(b"p"), Some(Command::GetPower))),
        (b"r", matches!(parse_command(b"r"), Some(Command::Reset))),
        (b"s", matches!(parse_command(b"s"), Some(Command::State))),
    ] {
        assert!(matches_command, "{bytes:?}");
    }
    assert!(matches!(parse_command(b"p0"), Some(Command::SetPower(0))));
    assert!(matches!(
        parse_command(b"p100"),
        Some(Command::SetPower(100))
    ));
    for bytes in [&b"p101"[..], b"p-1", b"p1.5", b"p01"] {
        assert!(parse_command(bytes).is_none());
    }
}

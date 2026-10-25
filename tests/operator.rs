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
    let mut active_config = SimulationConfig::default();
    let mut staged_config = active_config;
    scoped_reply(plant, &mut active_config, &mut staged_config, bytes).0
}

fn scoped_reply(
    plant: &mut ThermalPlant,
    active_config: &mut SimulationConfig,
    staged_config: &mut SimulationConfig,
    bytes: &[u8],
) -> (std::string::String, bool) {
    let mut output = Output::default();
    let restarted =
        handle_scoped_request(&mut output, bytes, plant, active_config, staged_config).unwrap();
    (output.0, restarted)
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
fn unknown_tokens_do_not_change_power_or_configuration() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    plant.set_power_percent(75.0).unwrap();
    for scope in ["o", "d"] {
        for key in [
            "poweq",
            "powez",
            "zzzzz",
            "devicz",
            "t.samplz",
            "t.heatez",
            "t.rooz",
            "room.capacitx",
            "seez",
            "debug.active.capacity",
        ] {
            for operation in ["get", "set"] {
                let value = if operation == "set" { " 50" } else { "" };
                let request = std::format!("{scope} 109 {operation} {key}{value}");
                assert_eq!(
                    scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
                    ("@109 err key\n".into(), false)
                );
                assert_eq!(plant.power_percent(), 75.0);
                assert_eq!(active.parameters.room_capacity, 300000.0);
                assert_eq!(staged.parameters.room_capacity, 300000.0);
                assert_eq!(active.seed, 1);
                assert_eq!(staged.seed, 1);
            }
        }
    }
    for token in ["applx", "applz", "xxxxx", "abcde", "resetx", "applyx"] {
        let request = std::format!("d 110 {token}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@110 err syntax\n".into(), false)
        );
        assert_eq!(plant.power_percent(), 75.0);
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
    for request in ["d 26 get unknown", "d 26 set heater.unknown 1"] {
        assert_eq!(reply(&mut plant, request.as_bytes()), "@26 err key\n");
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

fn assert_same_plant(actual: &ThermalPlant, expected: &ThermalPlant) {
    for (actual, expected) in [
        (actual.temperatures(), expected.temperatures()),
        (
            actual.measured_temperatures(),
            expected.measured_temperatures(),
        ),
    ] {
        assert_eq!(actual.heater_temp, expected.heater_temp);
        assert_eq!(actual.sample_temp, expected.sample_temp);
        assert_eq!(actual.room_temp, expected.room_temp);
    }
    assert_eq!(actual.power_percent(), expected.power_percent());
}

#[test]
fn designer_stages_both_keys_without_changing_running_plant() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    let mut control = ThermalPlant::new(active).unwrap();
    plant.set_power_percent(75.0).unwrap();
    control.set_power_percent(75.0).unwrap();
    for _ in 0..100 {
        plant.step();
        control.step();
    }
    for (request, response) in [
        ("d 30 get room.capacity", "@30 ok 300000\n"),
        ("d 31 set room.capacity 350000", "@31 ok\n"),
        ("d 32 get room.capacity", "@32 ok 350000\n"),
        ("d 33 set seed 12345", "@33 ok\n"),
        ("d 65535 get seed", "@65535 ok 12345\n"),
        ("d 36 get unknown", "@36 err key\n"),
        ("d 37 set unknown 1", "@37 err key\n"),
        ("d 38 get seed extra", "@38 err syntax\n"),
        ("o 39 get power", "@39 ok 75\n"),
    ] {
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            (response.into(), false)
        );
        assert_eq!(active.parameters.room_capacity, 300000.0);
        assert_eq!(active.seed, 1);
        assert_same_plant(&plant, &control);
    }
    assert_eq!(staged.parameters.room_capacity, 350000.0);
    assert_eq!(staged.seed, 12345);
    for _ in 0..100 {
        plant.step();
        control.step();
        assert_same_plant(&plant, &control);
    }
}

#[test]
fn designer_rejects_invalid_numbers_without_changing_configs() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    staged.parameters.room_capacity = 350000.0;
    staged.seed = 12345;
    let mut plant = ThermalPlant::new(active).unwrap();
    for key in ["room.capacity", "seed"] {
        for value in [
            "NaN", "inf", "1x", "1..0", "1e", "1e+", "1e-", "--1", "1,5", "1e100", "1e-100",
        ] {
            let request = std::format!("d 40 set {key} {value}");
            assert_eq!(
                scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
                ("@40 err value\n".into(), false)
            );
            assert_eq!(staged.parameters.room_capacity, 350000.0);
            assert_eq!(staged.seed, 12345);
            assert_eq!(active.parameters.room_capacity, 300000.0);
            assert_eq!(active.seed, 1);
        }
    }
    for value in ["16777217", "4294967295"] {
        let request = std::format!("d 41 set room.capacity {value}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@41 err value\n".into(), false)
        );
        assert_eq!(staged.parameters.room_capacity, 350000.0);
    }
    // Empty values are rejected by the existing request parser.
    for request in ["d 42 set seed ", "d 42 set room.capacity"] {
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@42 err syntax\n".into(), false)
        );
    }
    assert_eq!(staged.seed, 12345);
    assert_eq!(staged.parameters.room_capacity, 350000.0);
}

#[test]
fn designer_integer_boundaries_round_trip_exactly() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    for (key, values) in [
        ("seed", &[0u32, 1, u32::MAX][..]),
        (
            "room.capacity",
            &[0u32, 1, 16777216, 16777218, 4294967040][..],
        ),
    ] {
        for value in values {
            let request = std::format!("d 43 set {key} {value}");
            assert_eq!(
                scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
                ("@43 ok\n".into(), false)
            );
            let request = std::format!("d 44 get {key}");
            assert_eq!(
                scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
                (std::format!("@44 ok {value}\n"), false)
            );
        }
    }
    assert_eq!(active.seed, 1);
    assert_eq!(active.parameters.room_capacity, 300000.0);
}

#[test]
fn room_capacity_integer_form_preserves_stage2_contract() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    plant.set_power_percent(75.0).unwrap();
    for (text, accepted) in [
        ("350000", true),
        ("16777216", true),
        ("16777217", false),
        ("16777218", true),
        ("4294967040", true),
        ("4294967295", false),
        ("4294967296", false),
        ("+1", false),
        ("-1", false),
        ("+0", false),
        ("-0", false),
        ("0", true),
        ("000350000", true),
        ("0000", true),
        ("0004294967296", false),
    ] {
        let previous = staged.parameters.room_capacity.to_bits();
        let request = std::format!("d 101 set room.capacity {text}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            (
                if accepted {
                    "@101 ok\n"
                } else {
                    "@101 err value\n"
                }
                .into(),
                false
            ),
            "{text}"
        );
        if accepted {
            assert_eq!(
                staged.parameters.room_capacity.to_bits(),
                text.parse::<f32>().unwrap().to_bits()
            );
        } else {
            assert_eq!(
                staged.parameters.room_capacity.to_bits(),
                previous,
                "{text}"
            );
        }
        assert_eq!(active.parameters.room_capacity, 300000.0);
        assert_eq!(plant.power_percent(), 75.0);
    }
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 101 apply"),
        ("@101 err config\n".into(), false)
    );
    assert_eq!(plant.power_percent(), 75.0);
    for text in [
        "350000.5",
        "1e6",
        "4294967296.0",
        "4294967296e0",
        "+1.0",
        "-1e0",
        "16777217.0",
        "16777217e0",
    ] {
        let request = std::format!("d 102 set room.capacity {text}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@102 ok\n".into(), false),
            "{text}"
        );
        assert_eq!(
            staged.parameters.room_capacity.to_bits(),
            text.parse::<f32>().unwrap().to_bits()
        );
        assert_eq!(active.parameters.room_capacity, 300000.0);
        assert_eq!(plant.power_percent(), 75.0);
    }
    // Other f32 parameters still accept signed and wider integer forms.
    for text in ["+1", "-1", "4294967296"] {
        let request = std::format!("d 103 set heater.capacity {text}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@103 ok\n".into(), false)
        );
        assert_eq!(
            staged.parameters.heater_capacity.to_bits(),
            text.parse::<f32>().unwrap().to_bits()
        );
    }
}

#[test]
fn room_capacity_set_get_set_get_preserves_signed_integer_bits() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    let mut control = ThermalPlant::new(active).unwrap();
    plant.set_power_percent(75.0).unwrap();
    control.set_power_percent(75.0).unwrap();
    for _ in 0..20 {
        plant.step();
        control.step();
    }
    for (input, expected_text) in [
        ("-0.0", Some("-0.0")),
        ("-1.0", Some("-1.0")),
        ("-350000.0", Some("-350000.0")),
        ("-4294967040.0", Some("-4294967040.0")),
        ("0", Some("0")),
        ("350000", Some("350000")),
        ("350000.5", None),
    ] {
        let expected_bits = input.parse::<f32>().unwrap().to_bits();
        let request = std::format!("d 401 set room.capacity {input}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@401 ok\n".into(), false)
        );
        assert_eq!(staged.parameters.room_capacity.to_bits(), expected_bits);
        let (first, restarted) = scoped_reply(
            &mut plant,
            &mut active,
            &mut staged,
            b"d 402 get room.capacity",
        );
        assert!(!restarted);
        assert!(first.starts_with("@402 ok "));
        let text = first.split_whitespace().nth(2).unwrap();
        if let Some(expected_text) = expected_text {
            assert_eq!(text, expected_text);
        }
        assert_eq!(text.parse::<f32>().unwrap().to_bits(), expected_bits);
        let request = std::format!("d 403 set room.capacity {text}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@403 ok\n".into(), false)
        );
        assert_eq!(staged.parameters.room_capacity.to_bits(), expected_bits);
        let (second, restarted) = scoped_reply(
            &mut plant,
            &mut active,
            &mut staged,
            b"d 404 get room.capacity",
        );
        assert!(!restarted);
        assert!(second.starts_with("@404 ok "));
        let second_text = second.split_whitespace().nth(2).unwrap();
        assert_eq!(second_text, text);
        assert_eq!(second_text.parse::<f32>().unwrap().to_bits(), expected_bits);
        for invalid in ["-1", "+1"] {
            let request = std::format!("d 405 set room.capacity {invalid}");
            assert_eq!(
                scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
                ("@405 err value\n".into(), false)
            );
            assert_eq!(staged.parameters.room_capacity.to_bits(), expected_bits);
            assert_eq!(
                active.parameters.room_capacity.to_bits(),
                300000.0f32.to_bits()
            );
            assert_same_plant(&plant, &control);
        }
        if input.parse::<f32>().unwrap() <= 0.0 {
            assert_eq!(
                scoped_reply(&mut plant, &mut active, &mut staged, b"d 406 apply"),
                ("@406 err config\n".into(), false)
            );
            assert_eq!(staged.parameters.room_capacity.to_bits(), expected_bits);
        }
        assert_eq!(
            active.parameters.room_capacity.to_bits(),
            300000.0f32.to_bits()
        );
        assert_same_plant(&plant, &control);
    }
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 407 get seed"),
        ("@407 ok 1\n".into(), false)
    );
}

#[test]
fn designer_apply_replaces_plant_and_reset_reproduces_active_configuration() {
    let mut active = SimulationConfig::default();
    // Exercise seeded behavior without adding any further Designer keys.
    active.sensor_noise = 1.0;
    active.sensor_drift_amplitude = 0.5;
    active.parameter_spread = 0.05;
    active.heater_power_fluctuation = 0.05;
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    plant.set_power_percent(75.0).unwrap();
    for _ in 0..100 {
        plant.step();
    }
    for request in [
        b"d 31 set room.capacity 350000".as_slice(),
        b"d 33 set seed 12345",
    ] {
        assert!(!scoped_reply(&mut plant, &mut active, &mut staged, request).1);
    }
    let expected_config = staged;
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 34 apply"),
        ("@34 ok\n".into(), true)
    );
    assert_eq!(active.parameters.room_capacity, 350000.0);
    assert_eq!(active.seed, 12345);
    let mut expected = ThermalPlant::new(expected_config).unwrap();
    assert_same_plant(&plant, &expected);
    plant.set_power_percent(75.0).unwrap();
    expected.set_power_percent(75.0).unwrap();
    for _ in 0..150 {
        plant.step();
        expected.step();
        assert_same_plant(&plant, &expected);
    }
    // Pending invalid physical values must be discarded by reset.
    for request in [b"d 45 set room.capacity 0".as_slice(), b"d 46 set seed 999"] {
        assert!(!scoped_reply(&mut plant, &mut active, &mut staged, request).1);
    }
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 35 reset"),
        ("@35 ok\n".into(), true)
    );
    assert_eq!(staged.parameters.room_capacity, 350000.0);
    assert_eq!(staged.seed, 12345);
    assert_eq!(active.parameters.room_capacity, 350000.0);
    assert_eq!(active.seed, 12345);
    let mut expected = ThermalPlant::new(expected_config).unwrap();
    assert_same_plant(&plant, &expected);
    plant.set_power_percent(75.0).unwrap();
    expected.set_power_percent(75.0).unwrap();
    for _ in 0..150 {
        plant.step();
        expected.step();
        assert_same_plant(&plant, &expected);
    }
}

#[test]
fn designer_failed_apply_preserves_running_plant_and_pending_values() {
    let mut active = SimulationConfig::default();
    active.sensor_noise = 1.0;
    active.sensor_drift_amplitude = 0.5;
    active.heater_power_fluctuation = 0.05;
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    let mut control = ThermalPlant::new(active).unwrap();
    plant.set_power_percent(75.0).unwrap();
    control.set_power_percent(75.0).unwrap();
    for _ in 0..100 {
        plant.step();
        control.step();
    }
    for (request, response) in [
        ("d 47 set room.capacity 0", "@47 ok\n"),
        ("d 48 set seed 12345", "@48 ok\n"),
        ("d 49 apply", "@49 err config\n"),
        ("d 50 get room.capacity", "@50 ok 0\n"),
        ("d 51 get seed", "@51 ok 12345\n"),
        ("o 52 get power", "@52 ok 75\n"),
    ] {
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            (response.into(), false)
        );
        assert_same_plant(&plant, &control);
        assert_eq!(active.parameters.room_capacity, 300000.0);
        assert_eq!(active.seed, 1);
    }
    // Continued equality checks that failed apply also leaves random state intact.
    for _ in 0..150 {
        plant.step();
        control.step();
        assert_same_plant(&plant, &control);
    }
    assert_eq!(staged.parameters.room_capacity, 0.0);
    assert_eq!(staged.seed, 12345);
    assert_eq!(
        scoped_reply(
            &mut plant,
            &mut active,
            &mut staged,
            b"d 53 set room.capacity 350000"
        ),
        ("@53 ok\n".into(), false)
    );
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 54 apply"),
        ("@54 ok\n".into(), true)
    );
    assert_same_plant(&plant, &ThermalPlant::new(staged).unwrap());
}

#[test]
fn designer_apply_validates_the_entire_staged_configuration() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    staged.parameters.heater_capacity = 0.0;
    let mut plant = ThermalPlant::new(active).unwrap();
    let control = ThermalPlant::new(active).unwrap();
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 55 apply"),
        ("@55 err config\n".into(), false)
    );
    assert_same_plant(&plant, &control);
    assert_eq!(active.parameters.heater_capacity, 100.0);
    assert_eq!(staged.parameters.heater_capacity, 0.0);
}

#[test]
fn designer_reset_discards_valid_pending_capacity_after_apply() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    for (request, response, restarted) in [
        ("d 62 set room.capacity 350000", "@62 ok\n", false),
        ("d 63 apply", "@63 ok\n", true),
        ("d 64 set room.capacity 400000", "@64 ok\n", false),
        ("d 65 get room.capacity", "@65 ok 400000\n", false),
        ("d 66 reset", "@66 ok\n", true),
        ("d 67 get room.capacity", "@67 ok 350000\n", false),
    ] {
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            (response.into(), restarted)
        );
    }
    assert_eq!(active.parameters.room_capacity, 350000.0);
    assert_eq!(staged.parameters.room_capacity, 350000.0);
    assert_same_plant(&plant, &ThermalPlant::new(active).unwrap());
}

#[test]
fn designer_reset_succeeds_after_rejected_apply() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    for (request, response, restarted) in [
        ("d 62 set room.capacity 350000", "@62 ok\n", false),
        ("d 63 apply", "@63 ok\n", true),
        ("d 69 set room.capacity 0", "@69 ok\n", false),
        ("d 70 apply", "@70 err config\n", false),
        ("d 71 get room.capacity", "@71 ok 0\n", false),
        ("d 73 reset", "@73 ok\n", true),
        ("d 74 get room.capacity", "@74 ok 350000\n", false),
    ] {
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            (response.into(), restarted)
        );
    }
    assert_eq!(active.parameters.room_capacity, 350000.0);
    assert_eq!(staged.parameters.room_capacity, 350000.0);
    assert_same_plant(&plant, &ThermalPlant::new(active).unwrap());
}

#[test]
fn designer_all_defaults_are_readable() {
    let mut plant = ThermalPlant::new(SimulationConfig::default()).unwrap();
    for (key, expected) in [
        ("heater.capacity", 100.0),
        ("sample.capacity", 200.0),
        ("room.capacity", 300000.0),
        ("coupling", 2.0),
        ("heater.loss", 1.0),
        ("sample.loss", 0.5),
        ("room.loss", 20.0),
        ("heater.radiation", 4e-10),
        ("sample.radiation", 2e-10),
        ("heater.max_power", 100.0),
        ("initial.heater", 20.0),
        ("initial.sample", 20.0),
        ("initial.room", 20.0),
        ("background.temp", 20.0),
        ("parameter.spread", 0.0),
        ("power.fluctuation", 0.0),
        ("sensor.noise", 0.0),
        ("sensor.drift", 0.0),
    ] {
        let request = std::format!("d 301 get {key}");
        let response = reply(&mut plant, request.as_bytes());
        assert!(response.starts_with("@301 ok "));
        let value = response
            .split_whitespace()
            .nth(2)
            .unwrap()
            .parse::<f32>()
            .unwrap();
        assert_eq!(value, expected, "{key}");
    }
    assert_eq!(reply(&mut plant, b"d 302 get seed"), "@302 ok 1\n");
}

#[test]
fn designer_sets_all_parameters_then_applies_and_resets() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    let mut control = ThermalPlant::new(active).unwrap();
    plant.set_power_percent(75.0).unwrap();
    control.set_power_percent(75.0).unwrap();
    for _ in 0..100 {
        plant.step();
        control.step();
    }
    let values = [
        ("heater.capacity", "111.25"),
        ("sample.capacity", "222.5"),
        ("room.capacity", "350000.5"),
        ("coupling", "3.25"),
        ("heater.loss", "0.75"),
        ("sample.loss", "0.625"),
        ("room.loss", "22.5"),
        ("heater.radiation", "4e-10"),
        ("sample.radiation", "2.5e-7"),
        ("heater.max_power", "125.5"),
        ("initial.heater", "-10.75"),
        ("initial.sample", "20.25"),
        ("initial.room", "18.5"),
        ("background.temp", "17.25"),
        ("parameter.spread", "0.03"),
        ("power.fluctuation", "0.04"),
        ("sensor.noise", "0.5"),
        ("sensor.drift", "0.25"),
        ("seed", "12345"),
    ];
    for (key, text) in values {
        let request = std::format!("d 303 set {key} {text}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@303 ok\n".into(), false)
        );
        assert_same_plant(&plant, &control);
    }
    for (key, text) in values {
        let request = std::format!("d 304 get {key}");
        let (response, restarted) =
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes());
        assert!(!restarted);
        assert!(response.starts_with("@304 ok "));
        assert_eq!(
            response
                .split_whitespace()
                .nth(2)
                .unwrap()
                .parse::<f32>()
                .unwrap(),
            text.parse::<f32>().unwrap(),
            "{key}"
        );
        // Every complete round-trip request fits the unchanged UART buffer.
        assert!(
            std::format!(
                "d 65535 set {key} {}",
                response.split_whitespace().nth(2).unwrap()
            )
            .len()
                <= 64
        );
    }
    let mut expected = SimulationConfig::default();
    expected.parameters.heater_capacity = 111.25;
    expected.parameters.sample_capacity = 222.5;
    expected.parameters.room_capacity = 350000.5;
    expected.parameters.coupling = 3.25;
    expected.parameters.heater_loss = 0.75;
    expected.parameters.sample_loss = 0.625;
    expected.parameters.room_loss = 22.5;
    expected.parameters.heater_radiation = 4e-10;
    expected.parameters.sample_radiation = 2.5e-7;
    expected.parameters.max_heater_power = 125.5;
    expected.initial_state.heater_temp = -10.75;
    expected.initial_state.sample_temp = 20.25;
    expected.initial_state.room_temp = 18.5;
    expected.background_temp = 17.25;
    expected.parameter_spread = 0.03;
    expected.heater_power_fluctuation = 0.04;
    expected.sensor_noise = 0.5;
    expected.sensor_drift_amplitude = 0.25;
    expected.seed = 12345;
    assert_eq!(active.parameters.room_capacity, 300000.0);
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 305 apply"),
        ("@305 ok\n".into(), true)
    );
    let mut control = ThermalPlant::new(expected).unwrap();
    assert_same_plant(&plant, &control);
    plant.set_power_percent(75.0).unwrap();
    control.set_power_percent(75.0).unwrap();
    for _ in 0..100 {
        plant.step();
        control.step();
        assert_same_plant(&plant, &control);
    }
    for (key, _) in values {
        let request = std::format!("d 306 set {key} 0");
        assert!(!scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()).1);
    }
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 307 reset"),
        ("@307 ok\n".into(), true)
    );
    assert_same_plant(&plant, &ThermalPlant::new(expected).unwrap());
    for (key, text) in values {
        let request = std::format!("d 308 get {key}");
        let (response, _) = scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes());
        assert_eq!(
            response
                .split_whitespace()
                .nth(2)
                .unwrap()
                .parse::<f32>()
                .unwrap(),
            text.parse::<f32>().unwrap(),
            "{key}"
        );
    }
}

#[test]
fn designer_physical_rules_are_enforced_only_on_apply() {
    for (key, value) in [
        ("heater.capacity", "0"),
        ("sample.capacity", "-1"),
        ("room.capacity", "0"),
        ("coupling", "-0.1"),
        ("heater.loss", "-1"),
        ("sample.loss", "-1"),
        ("room.loss", "-1"),
        ("heater.radiation", "-4e-10"),
        ("sample.radiation", "-2e-10"),
        ("heater.max_power", "0"),
        ("initial.heater", "-273.15"),
        ("initial.sample", "-300"),
        ("initial.room", "-300"),
        ("background.temp", "-300"),
        ("parameter.spread", "0.11"),
        ("parameter.spread", "-0.01"),
        ("power.fluctuation", "0.11"),
        ("power.fluctuation", "-0.01"),
        ("sensor.noise", "2.001"),
        ("sensor.noise", "-0.01"),
        ("sensor.drift", "1.001"),
        ("sensor.drift", "-0.01"),
    ] {
        let mut active = SimulationConfig::default();
        let mut staged = active;
        let mut plant = ThermalPlant::new(active).unwrap();
        let mut control = ThermalPlant::new(active).unwrap();
        plant.set_power_percent(75.0).unwrap();
        control.set_power_percent(75.0).unwrap();
        for _ in 0..100 {
            plant.step();
            control.step();
        }
        let request = std::format!("d 310 set {key} {value}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@310 ok\n".into(), false),
            "{key}"
        );
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, b"d 311 apply"),
            ("@311 err config\n".into(), false),
            "{key}"
        );
        assert_same_plant(&plant, &control);
        for _ in 0..10 {
            plant.step();
            control.step();
            assert_same_plant(&plant, &control);
        }
        let request = std::format!("d 312 get {key}");
        let (response, _) = scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes());
        assert_eq!(
            response
                .split_whitespace()
                .nth(2)
                .unwrap()
                .parse::<f32>()
                .unwrap(),
            value.parse::<f32>().unwrap()
        );
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, b"d 313 reset"),
            ("@313 ok\n".into(), true)
        );
        assert_same_plant(
            &plant,
            &ThermalPlant::new(SimulationConfig::default()).unwrap(),
        );
    }
}

#[test]
fn designer_all_float_keys_reject_bad_input_and_near_misses() {
    let keys = [
        "heater.capacity",
        "sample.capacity",
        "room.capacity",
        "coupling",
        "heater.loss",
        "sample.loss",
        "room.loss",
        "heater.radiation",
        "sample.radiation",
        "heater.max_power",
        "initial.heater",
        "initial.sample",
        "initial.room",
        "background.temp",
        "parameter.spread",
        "power.fluctuation",
        "sensor.noise",
        "sensor.drift",
    ];
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    for key in keys {
        let request = std::format!("d 314 get {key}");
        let original = scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()).0;
        for value in [
            "abc", "NaN", "Inf", "Infinity", "1e100", "1e-100", "1e+", "1,5", "--1",
        ] {
            let request = std::format!("d 315 set {key} {value}");
            assert_eq!(
                scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
                ("@315 err value\n".into(), false),
                "{key}: {value}"
            );
        }
        let request = std::format!("d 314 get {key}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()).0,
            original
        );
        let mut miss = key.as_bytes().to_vec();
        *miss.last_mut().unwrap() = b'z';
        let miss = std::string::String::from_utf8(miss).unwrap();
        for request in [
            std::format!("d 316 get {miss}"),
            std::format!("d 316 set {miss} 1"),
        ] {
            assert_eq!(
                scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
                ("@316 err key\n".into(), false)
            );
        }
    }
    for value in ["-1", "+1", "1.5", "1e3", "4294967296"] {
        let request = std::format!("d 317 set seed {value}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@317 err value\n".into(), false)
        );
    }
}

#[test]
fn designer_accepts_valid_physical_boundaries() {
    let mut active = SimulationConfig::default();
    let mut staged = active;
    let mut plant = ThermalPlant::new(active).unwrap();
    for (key, value) in [
        ("heater.capacity", "1"),
        ("sample.capacity", "1"),
        ("room.capacity", "1.5"),
        ("coupling", "0"),
        ("heater.loss", "0"),
        ("sample.loss", "0"),
        ("room.loss", "0"),
        ("heater.radiation", "0"),
        ("sample.radiation", "0"),
        ("heater.max_power", "1"),
        ("initial.heater", "-273.14"),
        ("initial.sample", "-273.14"),
        ("initial.room", "-273.14"),
        ("background.temp", "-273.14"),
        ("parameter.spread", "0.1"),
        ("power.fluctuation", "0.1"),
        ("sensor.noise", "2"),
        ("sensor.drift", "1"),
        ("seed", "0"),
    ] {
        let request = std::format!("d 318 set {key} {value}");
        assert_eq!(
            scoped_reply(&mut plant, &mut active, &mut staged, request.as_bytes()),
            ("@318 ok\n".into(), false)
        );
    }
    assert_eq!(
        scoped_reply(&mut plant, &mut active, &mut staged, b"d 319 apply"),
        ("@319 ok\n".into(), true)
    );
    assert_same_plant(&plant, &ThermalPlant::new(staged).unwrap());
}

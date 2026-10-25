use super::*;

fn drain(
    receiver: &mut Receiver,
    partial: &mut std::vec::Vec<u8>,
) -> std::vec::Vec<Result<std::vec::Vec<u8>, ()>> {
    let mut lines = std::vec::Vec::new();
    while let Some(event) = receiver.pop() {
        match event {
            Err(()) => {
                partial.clear();
                lines.push(Err(()));
            }
            Ok(b'\r' | b'\n') => {
                if !partial.is_empty() {
                    lines.push(Ok(core::mem::take(partial)));
                }
            }
            Ok(byte) => partial.push(byte),
        }
    }
    lines
}

#[test]
fn ordinary_command_is_received_in_order() {
    let mut receiver = Receiver::new();
    for &byte in b"d 19 get heater.capacity\n" {
        receiver.push(byte, false);
    }
    assert_eq!(
        drain(&mut receiver, &mut std::vec::Vec::new()),
        [Ok(b"d 19 get heater.capacity".to_vec())]
    );
    assert_eq!(receiver.pop(), None);
}

#[test]
fn indices_wrap_repeatedly_without_reordering() {
    let mut receiver = Receiver::new();
    for round in 0..100u8 {
        for offset in 0..17u8 {
            receiver.push(round.wrapping_add(offset), false);
        }
        for offset in 0..17u8 {
            assert_eq!(receiver.pop(), Some(Ok(round.wrapping_add(offset))));
        }
        assert_eq!(receiver.pop(), None);
    }
}

#[test]
fn overflow_discards_queued_and_partial_data_until_boundary() {
    let mut receiver = Receiver::new();
    let mut partial = b"o 1 set power ".to_vec();
    for _ in 0..SIZE {
        receiver.push(b'9', false);
    }
    assert_eq!(receiver.pop(), None);
    receiver.push(b'0', false);
    receiver.push(b'\n', false);
    // A full next command may already be queued before the main loop resumes.
    for &byte in b"o 2 get power\n" {
        receiver.push(byte, false);
    }
    assert_eq!(
        drain(&mut receiver, &mut partial),
        [Err(()), Ok(b"o 2 get power".to_vec())]
    );
    assert!(partial.is_empty());
}

#[test]
fn clean_delimiter_that_overflows_queue_recovers_boundary() {
    let mut receiver = Receiver::new();
    for _ in 0..SIZE - 1 {
        receiver.push(b'x', false);
    }
    receiver.push(b'\r', false);
    receiver.push(b'\n', false);
    assert_eq!(drain(&mut receiver, &mut std::vec::Vec::new()), [Err(())]);
    receiver.push(b'i', false);
    receiver.push(b'\n', false);
    assert_eq!(
        drain(&mut receiver, &mut std::vec::Vec::new()),
        [Ok(b"i".to_vec())]
    );
}

#[test]
fn erroneous_delimiter_is_not_used_to_resynchronize() {
    let mut receiver = Receiver::new();
    receiver.push(b'\n', true);
    for &byte in b"o 1 set power 99" {
        receiver.push(byte, false);
    }
    assert_eq!(receiver.pop(), None);
    receiver.push(b'\n', false);
    assert_eq!(receiver.pop(), Some(Err(())));
    assert_eq!(receiver.pop(), None);
}

#[test]
fn repeated_errors_cannot_expose_a_partial_command() {
    let mut receiver = Receiver::new();
    receiver.push(b'x', true);
    receiver.push(b'\n', false);
    receiver.push(b'x', true);
    assert_eq!(receiver.pop(), Some(Err(())));
    receiver.push(b'y', true);
    receiver.push(b'\n', false);
    assert_eq!(receiver.pop(), Some(Err(())));
    assert_eq!(receiver.pop(), None);
}

#[test]
fn damaged_mutations_never_reach_handlers_and_next_command_works() {
    use crate::{SimulationConfig, ThermalPlant};
    struct Output(std::string::String);
    impl ufmt::uWrite for Output {
        type Error = core::convert::Infallible;
        fn write_str(&mut self, text: &str) -> Result<(), Self::Error> {
            self.0.push_str(text);
            Ok(())
        }
    }
    for command in [
        b"o 1 set power 50\n".as_slice(),
        b"d 2 set room.capacity 350000\n",
        b"d 3 set seed 12345\n",
        b"d 4 apply\n",
        b"d 5 reset\n",
    ] {
        for error_index in 0..command.len() {
            let mut active = SimulationConfig::default();
            let mut staged = active;
            staged.parameters.room_capacity = 400000.0;
            staged.seed = 99;
            let mut plant = ThermalPlant::new(active).unwrap();
            plant.set_power_percent(75.0).unwrap();
            plant.step();
            let before = plant.measured_temperatures();
            let mut receiver = Receiver::new();
            let mut partial = std::vec::Vec::new();
            let mut events = std::vec::Vec::new();
            for (index, &byte) in command.iter().enumerate() {
                receiver.push(byte, index == error_index);
                events.extend(drain(&mut receiver, &mut partial));
            }
            // If the delimiter itself was faulty, only the next clean delimiter
            // can restore a trustworthy boundary; no command is sent here.
            receiver.push(b'\n', false);
            events.extend(drain(&mut receiver, &mut partial));
            assert_eq!(events, [Err(())], "{command:?}, damaged byte {error_index}");
            assert_eq!(plant.power_percent(), 75.0);
            assert_eq!(active.parameters.room_capacity, 300000.0);
            assert_eq!(active.seed, 1);
            assert_eq!(staged.parameters.room_capacity, 400000.0);
            assert_eq!(staged.seed, 99);
            let after = plant.measured_temperatures();
            assert_eq!(before.heater_temp.to_bits(), after.heater_temp.to_bits());
            assert_eq!(before.sample_temp.to_bits(), after.sample_temp.to_bits());
            assert_eq!(before.room_temp.to_bits(), after.room_temp.to_bits());

            for &byte in b"o 99 set power 25\n" {
                receiver.push(byte, false);
            }
            let lines = drain(&mut receiver, &mut partial);
            let mut out = Output(std::string::String::new());
            assert_eq!(lines.len(), 1);
            crate::handle_scoped_request(
                &mut out,
                lines[0].as_ref().unwrap(),
                &mut plant,
                &mut active,
                &mut staged,
            )
            .unwrap();
            assert_eq!(out.0, "@99 ok\n");
            assert_eq!(plant.power_percent(), 25.0);
            assert_eq!(active.parameters.room_capacity, 300000.0);
            assert_eq!(staged.parameters.room_capacity, 400000.0);
        }
    }
}

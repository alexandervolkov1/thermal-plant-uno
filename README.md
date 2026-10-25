# Thermal plant for Arduino Uno

Firmware for an Arduino Uno / ATmega328P thermal plant emulator.
The physical simulation is provided by the pinned `thermal-plant-model`
Git dependency. No persistent configuration storage is used.

## Build and tests

Use the Rust toolchain pinned in `rust-toolchain.toml` and AVR GCC tools:

```powershell
cargo build --release --locked
./tests/host/run.ps1
```

The AVR target and CPU are configured in `.cargo/config.toml`.

## UART protocol

57600 baud, 8N1, ASCII text. End each command with CR or LF.
Use sequential request/response exchange; avoid pipelining commands.
Request IDs are decimal integers from 1 to 65535, without leading zeros.
The command buffer holds 64 bytes, excluding the line delimiter.

```text
o <id> get <key>
o <id> set power <percent>
d <id> get <key>
d <id> set <key> <value>
d <id> apply
d <id> reset
```

Operator keys: `device`, `t.heater`, `t.sample`, `t.room`, `power`.
Temperatures are measured sensor values in degrees Celsius.
Only `power` is writable through Operator: integer 0 through 100 percent.

Designer keys (all `f32` except `seed`, which is `u32`):

```text
heater.capacity    sample.capacity    room.capacity
coupling           heater.loss        sample.loss
room.loss          heater.radiation   sample.radiation
heater.max_power   initial.heater     initial.sample
initial.room       background.temp    seed
parameter.spread   power.fluctuation  sensor.noise
sensor.drift
```

Designer `get` reads staged configuration; `set` edits only that copy.
`apply` validates and constructs a replacement plant before committing it.
A rejected apply preserves the running plant and active configuration.
`reset` reconstructs the active configuration and discards staged edits.
Successful apply/reset restart simulation timing and reset power to zero.
Decimal input accepts fractions and exponents; output round-trips finite f32.
Plain integer `room.capacity` input must be unsigned u32 exactly representable
as f32. Fractional/exponent forms use nearest-even conversion.

```text
o 1 get device
@1 ok THERMAL_PLANT 2
o 2 set power 0
@2 ok
d 3 set room.capacity 350000
@3 ok
d 4 apply
@4 ok
d 5 get room.capacity
@5 ok 350000
```

Errors are `@<id> err syntax|key|value|config|unsupported` when correlated.
Input without a valid ID, or a damaged receive stream, returns `ERR`.
Nonfinite numbers, overflow and nonzero values rounded to zero are rejected.
Physically invalid staged values are rejected on apply.
The legacy single-letter UART protocol has been removed.

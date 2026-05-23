# Crystallized Time

A time-crystal-driven MIDI gate generator. Simulates one or two disordered classical spin chains under periodic Floquet drive and converts their dynamics into MIDI output for a eurorack synthesizer. The rhythm, pitch, and modulation signals all emerge from the simulation rather than being programmed directly.

This repository contains the first working component of a larger audiovisual installation. See [`crystallized_time.md`](crystallized_time.md) for the full project description.

---

## How it works

**Simulation.** The program runs a model of coupled spins (8 sites by default). A periodic kick is applied to the whole chain at regular intervals. Under the right conditions (appropriate disorder, coupling strength, and temperature), the chain enters a period-doubled phase: each spin flips at half the drive rate. Nothing in the code specifies this rhythm; it emerges from the physics.

**Readout.** Three observables are tracked simultaneously per chain. Selected sites are watched for zero-crossings of sigma_z. Domain walls, the boundaries between adjacent sites pointing in opposite directions, are tracked as mobile objects that appear, drift, and annihilate. The summed sigma_z over output sites is emitted as a continuous CC modulation stream, reflecting the chain's instantaneous bias between up and down spin populations.

**MIDI output.** Each zero-crossing produces a short gate pulse, useful for triggering envelopes and rhythmic events. The summed-sigma_z CC on a configurable channel and CC number gives a moving modulation source that tracks the collective state of the output sites. Each domain wall produces a held note: note-on when the wall is born, note-off when it annihilates. Wall motion is reported as a CC stream (position mapped to a configurable CC number) or as discrete repitching when the wall crosses semitone boundaries. A chain clock on its own channel derives from the chain's mean magnetization crossing zero, providing a beat signal that degrades naturally when the chain leaves the locked phase.

**Two chains.** A second chain (`chain_b`) can be configured with its own physics, seed, and routing. Chains optionally couple through a configurable shape (currently `mean_field_z`), allowing polyrhythmic interaction where each chain keeps its own period but influences the other.

---

## Running

```sh
cargo build --release
cargo run --release -- --list-ports     # see available MIDI outputs
cargo run --release -- --port 1         # run, sending to port 1
```

A MIDI destination is required. On Windows, install [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html) and create a virtual port. On macOS, use the IAC driver. For hardware, a USB MIDI-to-CV interface will appear directly in the port list.

All routing, tempo, physics, coupling, OSC, and input settings live in [`config.toml`](config.toml) in the project root.

A second binary, `sweep`, runs a parameter sweep over `(eps, J)` for a single chain and writes a CSV plus a classification summary. Useful for finding the locked-phase region before committing to a config.

```sh
cargo run --release --bin sweep -- --output sweep.csv
```

---

## Command-line flags

| Flag | Default | Description |
|---|---|---|
| `--list-ports` | | Print available MIDI output ports and exit. |
| `--port <N>` | `0` | Output port index. |
| `--list-input-ports` | | Print available MIDI input ports and exit. |
| `--input-port <N>` | | Input port index. Absent leaves the chain autonomous. |
| `--config <path>` | `config.toml` | Path to the TOML config file. |
| `--periods <N>` | `20000` | Number of drive periods to run. |
| `--tui` | | Enable the ratatui-based terminal dashboard.

Everything else (tempo, RNG seed, physics, MIDI routing, coupling, OSC, perturbation mapping) is set in the config file.

---

## TUI monitor

Pass `--tui` to launch a terminal dashboard inside the same window. The dashboard updates at ~30 fps and shows:

- **Magnetization scope** — per-chain mean σz over the last ~1000 ticks, as a line or scatter chart.
- **Event log** — gate triggers, clock pulses, wall creation/destruction, OSC and MIDI input activity.
- **Physics parameters** — live kT, epsilon, J, omega, and mean magnetization per chain; coupling strengths.
- **Voice pitch editor** — see and modify gate voice pitches in real time.

### Keybindings

| Key | Action |
|---|---|
| `q` / `Esc` | Quit. |
| `s` | Toggle scatter/line mode on the scope chart. |
| `r` | Toggle reference line. |
| `e` | Enter voice pitch edit mode (or exit, when already editing). |

### Voice pitch editor

Press `e` to enter edit mode. The voice row highlights the currently selected voice in yellow.

| Key | Action |
|---|---|
| `Up` / `Down` | Move between voices on the current chain. |
| `Tab` | Switch between chains (when chain B is active). |
| `+` / `-` | Raise or lower the selected pitch by 1 semitone (clamped to 0–127). |
| `e` / `Esc` | Exit edit mode. |

Pitch changes take effect immediately — the next gate event from that voice uses the updated pitch. Note names are displayed alongside raw MIDI numbers (e.g. `v0:48(C3)`).

---

## The config file

`config.toml` is loaded from the project root by default. A full example covering both chains:

```toml
[tempo]
bpm = 120

[osc]                                # optional
listen_port = 9000
send_address = "127.0.0.1:9001"
state_rate_hz = 50
enable_state = true

[physics]                            # optional shared block
kt = 0.1
eps = 0.01
j = 1.2
w = 2.0
n_sites = 8
ticks_per_period = 25
# kick_angle = 3.14159              # default pi (period-2); set 2.0944 for period-3

[chain_a]
seed = 47

[chain_a.physics]                    # optional per-chain override
kt = 0.01

[chain_a.gates]
voice_0 = { channel = 1, pitch = 48 }
voice_2 = { channel = 1, pitch = 52 }
voice_4 = { channel = 1, pitch = 55 }
voice_6 = { channel = 1, pitch = 59 }
gate_length_ms = 50

[chain_a.walls]
voice_0 = 5
voice_1 = 6
voice_2 = 7
voice_3 = 8
pitch_low = 36
pitch_high = 84
motion_cc = 1
repitch_on_move = false

[chain_a.clock]
channel = 16

[chain_a.modulation]                  # optional summed-sigma_z CC
enabled = true
# channel = 1                         # defaults to first gate voice channel
cc_number = 1

[chain_b]                            # optional second chain
seed = 83

[chain_b.physics]
kick_angle = 2.0944                  # target period-3

[chain_b.gates]
voice_0 = { channel = 9, pitch = 60 }

[chain_b.clock]
channel = 15

[coupling]                           # optional, requires both chains
shape = "mean_field_z"
strength = 0.1                       # convenience: sets both directions
# strength_ab = 0.1                  # or set explicitly
# strength_ba = 0.05

[input]                              # optional MIDI input perturbation
[input.perturbation]
base_note = 60
kind = "rotate"                      # "flip" | "rotate" | "field_spike"
axis = "x"
magnitude = 0.3
velocity_scale = 1.0

[chain_a.chords]                     # optional chord progression
enabled = true
advance_on = "wall_death"            # "clock" | "wall_death" | "period"
advance_every = 2
select = "magnetization"             # "sequential" | "magnetization" | "random"
sequence = [
    [48, 52, 55, 59],                # Cmaj7
    [45, 48, 52, 55],                # Am7
    [43, 47, 50, 53],                # Fmaj7
    [47, 50, 53, 57],                # G7
]
```

Channel numbers in the file are 1-based (`1..=16`); MIDI pitches are `0..=127`.

**Gate voices.** Each `voice_N` entry means "this voice listens to chain site `N`". The shorthand `voice_0 = 1` sets channel 1 with a default pitch. The full form `voice_0 = { channel = 1, pitch = 48 }` sets both. Voices that share a channel are mono (a new note retires the previous one). Voices on distinct channels are polyphonic.

**Wall voices.** A pool of channels named `voice_N`. The allocator picks the next free channel round-robin when a wall is created. When the pool is full, the oldest active wall yields its channel. Delete `[chain_a.walls]` to disable wall output.

**Clock.** A gate pulse on the configured channel every time the chain's mean magnetization crosses zero. In the locked phase this fires at a stable rate; near a phase boundary it becomes irregular; in the thermal phase it stops.

**Modulation CC.** The sum of sigma_z over the chain's output sites is sampled every tick, mapped linearly to `[0, 127]` centred at sum-zero (CC 64), and emitted on the configured channel and CC number. The emission is change-filtered (sent only when the CC value differs from the last sent value). This gives a continuous modulation stream that mirrors the collective state of the output sites -- all spins up pushes the CC to 127, all down to 0, balanced to 64. Useful for patching to filter cutoff, pulse width, or any destination that benefits from a physics-derived continuous control signal.

**Physics.** The shared `[physics]` block applies to every chain that doesn't define its own `[chain_x.physics]`. Per-chain blocks win when both are present. Fields are all optional inside a physics block; absent fields fall through to defaults. `kick_angle` controls the target sub-harmonic (pi for period-2, 2*pi/3 for period-3, pi/2 for period-4, etc.).

**Coupling.** When both chains are configured, an optional `[coupling]` section enables inter-chain influence. `shape = "mean_field_z"` adds each chain's mean magnetization as a uniform z-field on the other. `strength` sets both directions to the same value; `strength_ab` and `strength_ba` set them separately. The two forms are mutually exclusive. `site_paired` and `shared_drive` shapes parse but are not yet implemented (they log a one-time warning and run with no coupling).

**Input.** When `[input]` is present and `--input-port` is given, incoming MIDI notes perturb the chain. `kind = "flip"` negates sigma_z on the targeted site; `"rotate"` rotates the spin by `magnitude` radians around `axis`; `"field_spike"` adds a one-tick field of `magnitude` on `axis`. Velocity scales the magnitude (except for `flip`, which is binary). Site mapping is `(note - base_note) mod n_sites`. Input feeds only `chain_a`.

The loader validates the file before the program starts:

- Channel numbers must be in `1..=16`.
- No channel can be claimed by more than one signal across gates, walls, modulation, and clock, in either chain.
- `voice_N` indices must correspond to real chain sites (`N < n_sites`).
- Pitch and CC numbers must be in range.
- Coupling strengths must be in `0.0..=2.0`.

Errors name the offending entry, e.g.

```
config error: channel 16 is assigned to both chain_a.gates.voice_3 and chain_a.clock
```

---

## Chain clock

The program watches the mean magnetization of each chain and emits a gate pulse every time it crosses zero. In the locked phase this fires at a stable rate; near a phase boundary the pulse rate becomes irregular; in the thermal phase it stops. Feed this into a clock divider to use it as the master clock for downstream sequencers and oscillators.

---

## Domain walls

A domain wall is the boundary between adjacent sites with opposite spin direction. Walls exist as objects: they are born in pairs, drift along the chain, and annihilate in pairs. Their lifetime ranges from a few ticks (transient in the locked phase) to many drive periods (persistent walls in the partially-disordered regime).

Each wall sounds as a held note on one of the wall channels, with note-on at birth and note-off at annihilation. Wall motion is reported in one of two ways, controlled by `repitch_on_move` in the config:

- **Held pitch + CC** (default, `repitch_on_move = false`). The note holds a single pitch; a CC stream tracks the wall's current position. Patch this CC to filter cutoff, pan, or any other modulation destination on the receiving synth.
- **Repitch on move** (`repitch_on_move = true`). Wall motion produces new note-on/note-off pairs as the position-derived pitch crosses semitone boundaries. Useful for hardware MIDI-to-CV interfaces where CC-to-modulation routing is inconvenient.

When the number of active walls exceeds the available channels, the oldest active wall yields its channel to the new one.

---

## Chord changes

Each chain can carry its own chord progression that rewrites gate voice pitches as the simulation runs. The progression is a list of voicings, advanced by a chosen physics event (clock pulse, wall destruction, or drive-period boundary). The next chord is selected sequentially, by chain magnetization, or by a tick-seeded RNG — so the same seed reproduces the same progression.

Each voicing must list one pitch per gate voice. When the progression advances, the new voicing is written into the same `voice_pitches` buffer the TUI editor edits, so the next gate event from that voice plays the new note immediately. Live editing in the TUI and chord changes share the same buffer; whichever wrote last wins.

```toml
[chain_a.chords]
enabled = true
advance_on = "wall_death"            # "clock" | "wall_death" | "period"
advance_every = 2                    # advance every Nth matching event
select = "magnetization"             # "sequential" | "magnetization" | "random"

# One pitch per gate voice. With four voices on chain_a, every voicing is length-4.
sequence = [
    [48, 52, 55, 59],                # Cmaj7
    [45, 48, 52, 55],                # Am7
    [43, 47, 50, 53],                # Fmaj7
    [47, 50, 53, 57],                # G7
    [40, 44, 47, 51],                # Em7
    [38, 41, 45, 48],                # Dm7
]
```

- `advance_on = "clock"` advances on every chain clock pulse — a beat-locked progression in the locked phase.
- `advance_on = "wall_death"` advances when a domain wall annihilates — sparse, event-driven changes that follow the physics.
- `advance_on = "period"` advances every `ticks_per_period` ticks — a strict metronomic progression independent of chain state.
- `select = "sequential"` walks the sequence round-robin.
- `select = "magnetization"` picks `floor(((m + 1) / 2) * n)` — chord index follows the chain's collective bias.
- `select = "random"` samples a tick-seeded LCG, so a given run reproduces the same chord order.

Validation: `sequence` must be non-empty when enabled, every voicing must match the chain's gate voice count, and pitches must be `0..=127`. The two chains' progressions run independently.

---

## Live control via OSC

The OSC layer turns the program into a system that can be shaped in real time. An external program (TouchDesigner, a Python script, a custom controller) can write to mutable parameters while the simulation runs. The program publishes its internal state and events back over a second port.

Both directions are opt-in. Without `listen_port` and `send_address` in the `[osc]` section, no OSC threads start.

### Inbound parameter control

When `listen_port` is set, the program accepts messages on the following addresses.

Physics parameters, written to both chains at once:

```
/physics/kt   float    effective temperature,  0.0 - 2.0
/physics/eps  float    drive imperfection,     0.0 - 0.5
/physics/j    float    coupling strength,      0.0 - 3.0
/physics/w    float    disorder width,         0.0 - 5.0
```

Per-chain variants, with the same value ranges:

```
/a/physics/kt, /a/physics/eps, /a/physics/j, /a/physics/w
/b/physics/kt, /b/physics/eps, /b/physics/j, /b/physics/w
```

Inter-chain coupling (only effective when both chains and `[coupling]` are configured):

```
/coupling/strength_ab   float    chain A's influence on chain B, 0.0 - 2.0
/coupling/strength_ba   float    chain B's influence on chain A, 0.0 - 2.0
/coupling/strength      float    writes both directions at once
```

Out-of-range values are clamped silently. Parameter changes are smoothed over several seconds (per-parameter time constants) so that sudden input produces a gradual transition rather than an audible click. Malformed packets are dropped silently.

### Outbound events and state

When `send_address` is set, the program publishes two kinds of traffic, namespaced by chain (`/a/...` for chain A, `/b/...` for chain B).

**Events** fire once per occurrence:

```
/a/site/event      int site, int voice, float intensity
/a/clock/pulse     float magnetization
/a/wall/created    int id, float position, int channel
/a/wall/destroyed  int id, float last_position, int lifetime_ticks
/a/wall/moved      int id, float from, float to, float velocity
```

(and the matching `/b/...` set when chain B is active).

**State** is sampled at up to `state_rate_hz` (default 50, set to 0 or `enable_state = false` to disable):

```
/a/state/spins           n_sites floats    per-site sigma_z values
/a/state/magnetization   float             mean sigma_z
/a/state/wall_count      int               number of active walls
```

Events and state that fire on the same tick are packed into a single OSC bundle and sent as one UDP packet.

### Typical invocation

```toml
[osc]
listen_port = 9000
send_address = "127.0.0.1:9001"
```

```sh
cargo run --release -- --port 1
```

The program listens for parameter writes on port 9000 and publishes to port 9001.

---

## MIDI input

When `[input]` is in the config and `--input-port <N>` is passed, incoming MIDI notes on the chosen port become localized perturbations on `chain_a`. Each note-on lands on one site (chosen by `(note - base_note) mod n_sites`) with magnitude scaled by velocity. The perturbation kind (`flip`, `rotate`, `field_spike`) is fixed per run by the config. Note-off messages and CC messages are ignored.

`--list-input-ports` prints the available input ports and exits.

---

## OSC message reference

**Inbound** (`listen_port`):

| Address | Arguments |
|---|---|
| `/physics/kt`, `/physics/eps`, `/physics/j`, `/physics/w` | float |
| `/a/physics/...`, `/b/physics/...` | float (per-chain variants) |
| `/coupling/strength_ab`, `/coupling/strength_ba` | float |
| `/coupling/strength` | float (writes both) |

**Outbound events** (`send_address`, per chain with `/a/` or `/b/` prefix):

| Address | Arguments |
|---|---|
| `/{chain}/site/event` | int site, int voice, float intensity |
| `/{chain}/clock/pulse` | float magnetization |
| `/{chain}/wall/created` | int id, float position, int channel |
| `/{chain}/wall/destroyed` | int id, float last_position, int lifetime_ticks |
| `/{chain}/wall/moved` | int id, float from, float to, float velocity |

**Outbound state** (throttled, per chain):

| Address | Arguments |
|---|---|
| `/{chain}/state/spins` | n_sites x float |
| `/{chain}/state/magnetization` | float |
| `/{chain}/state/wall_count` | int |

---

## License

MIT. See [`LICENSE`](LICENSE).
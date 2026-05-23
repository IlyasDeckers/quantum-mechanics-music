//! TOML configuration front-end.
//!
//! Reads a config file from disk, validates it, and produces a `Config`
//! struct the rest of the program already knows how to consume. The
//! existing `Config` types are not changed by this module — it is purely
//! a deserialization layer.
//!
//! Channel and pitch numbers in the file are 1-based for channels
//! (1..=16) and 0..=127 for MIDI pitches. Internal representations
//! convert channels to 0-based.

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::{ChainConfig, ChordConfig, ChordSelect, ChordTriggerKind, ClockConfig, Config, CouplingConfig, CouplingShape, EventConfig, InputConfig, MidiConfig, ModulationConfig, OscConfig, PerturbationConfig, PerturbationKindConfig, PhysicsConfig, PhysicsTargets, QuantizeConfig, TempoConfig, WallConfig, WallMidiConfig};
use crate::quantizer::Scale;

// --- Public API ------------------------------------------------------------

/// Top-level error for config-file problems. Variants distinguish IO
/// failures (file doesn't exist, can't read) from parse failures
/// (TOML syntax) from validation failures (semantically invalid).
#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    Validation(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io { path, source } => {
                write!(f, "could not read '{}': {}", path.display(), source)
            }
            ConfigError::Parse { path, source } => {
                write!(f, "could not parse '{}': {}", path.display(), source)
            }
            ConfigError::Validation(msg) => write!(f, "config error: {}", msg),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Read the TOML file at `path`, parse it, validate it, and return the
/// runtime `Config`. The single entry point this module exposes to
/// the rest of the program.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let toml_config: TomlConfig = toml::from_str(&raw).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        source: Box::new(e),
    })?;

    toml_config.into_config()
}

// --- TOML schema -----------------------------------------------------------
//
// One struct per TOML section. `Option<T>` fields are genuinely optional
// in the file; non-Option fields are required and serde will emit a
// parse error if they're missing. Defaults are applied at conversion
// time in `into_config`, not here, so that missing fields produce
// clearer errors than serde's default-substitution would.

#[derive(Debug, Deserialize)]
struct TomlConfig {
    tempo: Option<TempoSection>,
    osc: Option<OscSection>,
    physics: Option<PhysicsSection>,
    input: Option<InputSection>,
    coupling: Option<CouplingSection>,
    chain_a: ChainSection,
    chain_b: Option<ChainSection>,
}

#[derive(Debug, Deserialize)]
struct TempoSection {
    bpm: f64,
}

#[derive(Debug, Deserialize)]
struct OscSection {
    listen_port: Option<u16>,
    send_address: Option<String>,
    state_rate_hz: Option<f64>,
    state_spins_rate_hz: Option<f64>,
    enable_state: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct InputSection {
    perturbation: Option<PerturbationSection>,
}

#[derive(Debug, Deserialize)]
struct CouplingSection {
    shape: String,
    /// Convenience: when given, sets both strengths to this value.
    /// Mutually exclusive with strength_ab and strength_ba.
    strength: Option<f64>,
    strength_ab: Option<f64>,
    strength_ba: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct PerturbationSection {
    base_note: Option<u8>,
    velocity_scale: Option<f64>,
    /// One of "flip", "rotate", "field_spike". Defaults to "rotate".
    kind: Option<String>,
    /// Required for Rotate and FieldSpike. One of "x", "y", "z".
    axis: Option<String>,
    /// Rotation angle in radians (Rotate) or field magnitude (FieldSpike).
    magnitude: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
struct PhysicsSection {
    kt: Option<f64>,
    eps: Option<f64>,
    j: Option<f64>,
    w: Option<f64>,
    n_sites: Option<usize>,
    ticks_per_period: Option<u32>,
    dt: Option<f64>,
    kick_angle: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ChainSection {
    seed: Option<u64>,
    physics: Option<PhysicsSection>,
    gates: GatesSection,
    walls: Option<WallsSection>,
    clock: ClockSection,
    modulation: Option<ModulationSection>,
    quantize: Option<QuantizeSection>,
    chords: Option<ChordsSection>,
}

#[derive(Debug, Deserialize)]
struct ChordsSection {
    enabled: Option<bool>,
    advance_on: Option<String>,   // "clock" | "wall_death" | "period"
    advance_every: Option<u32>,
    select: Option<String>,       // "sequential" | "magnetization" | "random"
    sequence: Option<Vec<Vec<u8>>>,
}

#[derive(Debug, Deserialize)]
struct GatesSection {
    /// Optional gate length override for this chain.
    gate_length_ms: Option<u64>,
    /// All other fields — the `voice_N` entries — are captured as a
    /// flat map so we can accept any number of them and validate them
    /// uniformly.
    #[serde(flatten)]
    voices: BTreeMap<String, GateVoiceEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GateVoiceEntry {
    /// Shorthand: just the channel number. Pitch falls back to default.
    ChannelOnly(u8),
    /// Full form: channel and pitch named.
    Full { channel: u8, pitch: Option<u8> },
}

#[derive(Debug, Deserialize)]
struct WallsSection {
    pitch_low: Option<u8>,
    pitch_high: Option<u8>,
    /// 0 disables the motion CC. Any other value sets the CC number.
    motion_cc: Option<u8>,
    repitch_on_move: Option<bool>,
    /// `voice_N` entries — same flatten pattern as gates.
    #[serde(flatten)]
    voices: BTreeMap<String, WallVoiceEntry>,
}

/// Wall voice entries are always a bare channel number — wall pitch is
/// derived from position, not assigned per voice. Unlike gates there's
/// no struct-form variant.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WallVoiceEntry {
    Channel(u8),
}

#[derive(Debug, Deserialize)]
struct ClockSection {
    channel: u8,
    enabled: Option<bool>,
    pitch: Option<u8>,
    gate_length_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ModulationSection {
    enabled: Option<bool>,
    channel: Option<u8>,
    cc_number: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct QuantizeSection {
    scale: Option<String>,
    root_note: Option<u8>,
}

// --- Conversion ------------------------------------------------------------

impl TomlConfig {
    fn into_config(self) -> Result<Config, ConfigError> {
        let tempo = match self.tempo {
            Some(t) => TempoConfig::from_bpm(t.bpm),
            None => TempoConfig::default(),
        };
        let osc = build_osc(self.osc);

        let shared_physics = self.physics.as_ref();

        let (chain_a, claims_a) = build_chain(&self.chain_a, "chain_a", shared_physics)?;

        let (chain_b, claims_b) = match &self.chain_b {
            Some(section) => {
                let (cfg, claims) = build_chain(section, "chain_b", shared_physics)?;
                if cfg.physics.ticks_per_period != chain_a.physics.ticks_per_period {
                    return Err(ConfigError::Validation(
                        "chain_b.physics.ticks_per_period must match chain_a.physics.ticks_per_period".to_string(),
                    ));
                }
                (Some(cfg), claims)
            }
            None => (None, Vec::new()),
        };

        let input = build_input(self.input)?;
        let coupling = build_coupling(self.coupling)?;

        // Combined duplicate-channel validation across both chains.
        let mut all_claims = claims_a;
        all_claims.extend(claims_b);
        validate_no_duplicates(&all_claims)?;

        Ok(Config {
            chain_a,
            chain_b,
            coupling,
            tempo,
            osc,
            input,
        })
    }
}

fn build_osc(section: Option<OscSection>) -> OscConfig {
    let defaults = OscConfig::default();
    match section {
        Some(s) => OscConfig {
            listen_port: s.listen_port,
            send_address: s.send_address,
            state_rate_hz: s.state_rate_hz.unwrap_or(defaults.state_rate_hz),
            state_spins_rate_hz: s
                .state_spins_rate_hz
                .unwrap_or(defaults.state_spins_rate_hz),
            enable_state: s.enable_state.unwrap_or(defaults.enable_state),
        },
        None => defaults,
    }
}

fn build_input(section: Option<InputSection>) -> Result<Option<InputConfig>, ConfigError> {
    let Some(s) = section else { return Ok(None) };

    let pert_defaults = PerturbationConfig::default();
    let p = s.perturbation.unwrap_or(PerturbationSection {
        base_note: None,
        velocity_scale: None,
        kind: None,
        axis: None,
        magnitude: None,
    });

    let kind_str = p.kind.as_deref().unwrap_or("rotate").to_lowercase();
    let kind = match kind_str.as_str() {
        "flip" => PerturbationKindConfig::Flip,
        "rotate" => {
            let axis = parse_axis(p.axis.as_deref().unwrap_or("x"))?;
            let base_angle = p.magnitude.unwrap_or(0.3);
            PerturbationKindConfig::Rotate { axis, base_angle }
        }
        "field_spike" => {
            let axis = parse_axis(p.axis.as_deref().unwrap_or("x"))?;
            let base_magnitude = p.magnitude.unwrap_or(0.5);
            PerturbationKindConfig::FieldSpike { axis, base_magnitude }
        }
        other => {
            return Err(ConfigError::Validation(format!(
                "input.perturbation.kind must be 'flip', 'rotate', or 'field_spike' (got '{}')",
                other
            )));
        }
    };

    Ok(Some(InputConfig {
        perturbation: PerturbationConfig {
            base_note: p.base_note.unwrap_or(pert_defaults.base_note),
            kind,
            velocity_scale: p.velocity_scale.unwrap_or(pert_defaults.velocity_scale),
        },
    }))
}

fn parse_axis(s: &str) -> Result<crate::chain::Axis, ConfigError> {
    match s.to_lowercase().as_str() {
        "x" => Ok(crate::chain::Axis::X),
        "y" => Ok(crate::chain::Axis::Y),
        "z" => Ok(crate::chain::Axis::Z),
        other => Err(ConfigError::Validation(format!(
            "axis must be 'x', 'y', or 'z' (got '{}')",
            other
        ))),
    }
}

fn build_physics(section: Option<PhysicsSection>) -> PhysicsConfig {
    let defaults = PhysicsConfig::default();
    let Some(s) = section else { return defaults };

    PhysicsConfig {
        kt: PhysicsTargets::clamp_kt(s.kt.unwrap_or(defaults.kt)),
        eps: PhysicsTargets::clamp_eps(s.eps.unwrap_or(defaults.eps)),
        j: PhysicsTargets::clamp_j(s.j.unwrap_or(defaults.j)),
        w: PhysicsTargets::clamp_w(s.w.unwrap_or(defaults.w)),
        n_sites: s.n_sites.unwrap_or(defaults.n_sites),
        ticks_per_period: s.ticks_per_period.unwrap_or(defaults.ticks_per_period),
        dt: s.dt.unwrap_or(defaults.dt),
        kick_angle: s.kick_angle.unwrap_or(defaults.kick_angle),
    }
}

fn build_coupling(
    section: Option<CouplingSection>,
) -> Result<Option<CouplingConfig>, ConfigError> {
    let Some(s) = section else { return Ok(None) };

    // Shape parsing. Unknown shapes are rejected.
    let shape = match s.shape.to_lowercase().as_str() {
        "mean_field_z" => CouplingShape::MeanFieldZ,
        "site_paired"  => CouplingShape::SitePaired,
        "shared_drive" => CouplingShape::SharedDrive,
        other => {
            return Err(ConfigError::Validation(format!(
                "coupling.shape must be 'mean_field_z', 'site_paired', or 'shared_drive' (got '{}')",
                other
            )));
        }
    };

    // Strength resolution. Three valid input shapes:
    //   1. `strength` alone        -> both directions equal.
    //   2. `strength_ab` and/or `strength_ba` -> set those, default unset to 0.
    //   3. Nothing                 -> both default to 0.
    // Mixing `strength` with `strength_ab` or `strength_ba` is a config
    // error, because it makes the intended value of strength ambiguous.
    let has_combined = s.strength.is_some();
    let has_split    = s.strength_ab.is_some() || s.strength_ba.is_some();
    if has_combined && has_split {
        return Err(ConfigError::Validation(
            "coupling.strength is mutually exclusive with strength_ab / strength_ba".to_string(),
        ));
    }

    let (strength_ab, strength_ba) = if let Some(g) = s.strength {
        (g, g)
    } else {
        (s.strength_ab.unwrap_or(0.0), s.strength_ba.unwrap_or(0.0))
    };

    // Validate ranges. Same bounds as physics targets so OSC clamps and
    // file-loaded values agree.
    let validate = |name: &str, v: f64| -> Result<(), ConfigError> {
        if !(0.0..=2.0).contains(&v) {
            return Err(ConfigError::Validation(format!(
                "coupling.{} must be in 0.0..=2.0 (got {})",
                name, v
            )));
        }
        Ok(())
    };
    validate("strength_ab", strength_ab)?;
    validate("strength_ba", strength_ba)?;

    Ok(Some(CouplingConfig {
        shape,
        strength_ab,
        strength_ba,
    }))
}

/// Build `MidiConfig` from `[chain_a.gates]`. Returns the config plus
/// the list of (voice_index, channel) pairs claimed, for the duplicate
/// validator.
fn build_midi(
    section: &GatesSection,
    chain_label: &str,
) -> Result<(MidiConfig, Vec<ChannelClaim>), ConfigError> {
    let defaults = MidiConfig::default();
    let sorted = parse_voice_indices(&section.voices, &format!("{}.gates", chain_label))?;

    if sorted.is_empty() {
        return Err(ConfigError::Validation(format!(
            "{}.gates must define at least one voice_N",
            chain_label
        )));
    }

    let mut voice_channels = Vec::with_capacity(sorted.len());
    let mut voice_pitches = Vec::with_capacity(sorted.len());
    let mut claims = Vec::with_capacity(sorted.len());

    let default_pitches: &[u8] = &defaults.voice_pitches;

    for (idx, name, entry) in sorted {
        let (channel_1b, pitch) = match entry {
            GateVoiceEntry::ChannelOnly(c) => (*c, None),
            GateVoiceEntry::Full { channel, pitch } => (*channel, *pitch),
        };

        let label = format!("{}.gates.{}", chain_label, name);
        let channel_0b = validate_channel_1based(channel_1b, &label)?;
        let pitch = pitch
            .or_else(|| default_pitches.get(idx).copied())
            .unwrap_or(48);

        voice_channels.push(channel_0b);
        voice_pitches.push(pitch);
        claims.push(ChannelClaim {
            channel: channel_0b,
            label,
        });
    }

    let midi = MidiConfig {
        voice_channels,
        voice_pitches,
        gate_length_ms: section.gate_length_ms.unwrap_or(defaults.gate_length_ms),
    };

    Ok((midi, claims))
}

fn build_walls(
    section: Option<&WallsSection>,
    chain_label: &str,
) -> Result<(WallConfig, WallMidiConfig, Vec<ChannelClaim>), ConfigError> {
    let wall_defaults = WallConfig::default();
    let midi_defaults = WallMidiConfig::default();

    let Some(s) = section else {
        return Ok((
            WallConfig { enabled: false, ..wall_defaults },
            WallMidiConfig { channels: Vec::new(), ..midi_defaults },
            Vec::new(),
        ));
    };

    let sorted = parse_voice_indices(&s.voices, &format!("{}.walls", chain_label))?;

    let mut channels = Vec::with_capacity(sorted.len());
    let mut claims = Vec::with_capacity(sorted.len());
    for (_idx, name, entry) in sorted {
        let WallVoiceEntry::Channel(c) = entry;
        let claim_label = format!("{}.walls.{}", chain_label, name);
        let channel_0b = validate_channel_1based(*c, &claim_label)?;
        channels.push(channel_0b);
        claims.push(ChannelClaim {
            channel: channel_0b,
            label: claim_label,
        });
    }

    let motion_cc = match s.motion_cc {
        Some(0) => None,
        Some(n) if n <= 127 => Some(n),
        Some(n) => {
            return Err(ConfigError::Validation(format!(
                "{}.walls.motion_cc must be in 0..=127 (got {})",
                chain_label, n
            )));
        }
        None => midi_defaults.motion_cc,
    };

    let pitch_low = s.pitch_low.unwrap_or(midi_defaults.pitch_low);
    let pitch_high = s.pitch_high.unwrap_or(midi_defaults.pitch_high);
    if pitch_low > 127 || pitch_high > 127 {
        return Err(ConfigError::Validation(format!(
            "{}.walls pitch range must be in 0..=127 (got {}..{})",
            chain_label, pitch_low, pitch_high
        )));
    }
    if pitch_low > pitch_high {
        return Err(ConfigError::Validation(format!(
            "{}.walls pitch_low must be <= pitch_high (got {} > {})",
            chain_label, pitch_low, pitch_high
        )));
    }

    let enabled = !channels.is_empty();

    let wall_midi = WallMidiConfig {
        channels,
        pitch_low,
        pitch_high,
        motion_cc,
        repitch_on_move: s.repitch_on_move.unwrap_or(midi_defaults.repitch_on_move),
    };

    let walls = WallConfig {
        enabled,
        ..wall_defaults
    };

    Ok((walls, wall_midi, claims))
}

fn build_clock(
    section: &ClockSection,
    chain_label: &str,
) -> Result<ClockConfig, ConfigError> {
    let defaults = ClockConfig::default();
    let channel_0b = validate_channel_1based(
        section.channel,
        &format!("{}.clock.channel", chain_label),
    )?;
    Ok(ClockConfig {
        enabled: section.enabled.unwrap_or(defaults.enabled),
        channel: channel_0b,
        pitch: section.pitch.unwrap_or(defaults.pitch),
        crossing_threshold: defaults.crossing_threshold,
        debounce_ticks: defaults.debounce_ticks,
        gate_length_ms: section.gate_length_ms.unwrap_or(defaults.gate_length_ms),
    })
}

fn build_modulation(
    section: &Option<ModulationSection>,
    chain_label: &str,
    fallback_channel: Option<u8>,
) -> Result<ModulationConfig, ConfigError> {
    let defaults = ModulationConfig::default();
    let Some(s) = section else { return Ok(defaults) };
    let enabled = s.enabled.unwrap_or(defaults.enabled);
    if !enabled {
        return Ok(ModulationConfig { enabled: false, ..defaults });
    }
    let channel_1b = s.channel
        .or_else(|| fallback_channel.map(|c| c + 1))
        .unwrap_or(1);
    let channel_0b = validate_channel_1based(
        channel_1b,
        &format!("{}.modulation.channel", chain_label),
    )?;
    let cc_number = match s.cc_number {
        Some(n) if n <= 127 => n,
        Some(n) => {
            return Err(ConfigError::Validation(format!(
                "{}.modulation.cc_number must be in 0..=127 (got {})",
                chain_label, n
            )));
        }
        None => defaults.cc_number,
    };
    Ok(ModulationConfig {
        enabled: true,
        channel: channel_0b,
        cc_number,
    })
}

fn build_quantize(
    section: &Option<QuantizeSection>,
    chain_label: &str,
) -> Result<QuantizeConfig, ConfigError> {
    let defaults = QuantizeConfig::default();
    let Some(s) = section else { return Ok(defaults) };
    let scale = match &s.scale {
        Some(name) => Scale::from_name(name).ok_or_else(|| {
            ConfigError::Validation(format!(
                "{}.quantize.scale: unknown scale '{}' \
                 (valid: unquantized, major, minor, pentatonic, hirajoshi, iwato)",
                chain_label, name
            ))
        })?,
        None => defaults.scale,
    };
    let root_note = match s.root_note {
        Some(n) if n <= 127 => n,
        Some(n) => {
            return Err(ConfigError::Validation(format!(
                "{}.quantize.root_note must be in 0..=127 (got {})",
                chain_label, n
            )));
        }
        None => defaults.root_note,
    };
    Ok(QuantizeConfig { scale, root_note })
}

fn build_events(
    section: &GatesSection,
    chain_label: &str,
) -> Result<EventConfig, ConfigError> {
    let defaults = EventConfig::default();
    let sorted = parse_voice_indices(&section.voices, &format!("{}.gates", chain_label))?;
    let output_sites: Vec<usize> = sorted.iter().map(|(idx, _, _)| *idx).collect();
    Ok(EventConfig {
        output_sites,
        crossing_threshold: defaults.crossing_threshold,
        debounce_ticks: defaults.debounce_ticks,
    })
}

fn build_chords(
    section: &Option<ChordsSection>,
    n_voices: usize,
    chain_label: &str,
) -> Result<ChordConfig, ConfigError> {
    let Some(s) = section else { return Ok(ChordConfig::default()) };
    let enabled = s.enabled.unwrap_or(false);
    if !enabled { return Ok(ChordConfig::default()); }

    let advance_on = match s.advance_on.as_deref().unwrap_or("wall_death") {
        "clock"      => ChordTriggerKind::Clock,
        "wall_death" => ChordTriggerKind::WallDeath,
        "period"     => ChordTriggerKind::Period,
        other => return Err(ConfigError::Validation(format!(
            "{}.chords.advance_on: unknown '{}' (clock|wall_death|period)", chain_label, other
        ))),
    };

    let select = match s.select.as_deref().unwrap_or("sequential") {
        "sequential"    => ChordSelect::Sequential,
        "magnetization" => ChordSelect::Magnetization,
        "random"        => ChordSelect::Random,
        other => return Err(ConfigError::Validation(format!(
            "{}.chords.select: unknown '{}' (sequential|magnetization|random)", chain_label, other
        ))),
    };

    let sequence = s.sequence.clone().unwrap_or_default();

    if sequence.is_empty() {
        return Err(ConfigError::Validation(format!(
            "{}.chords.sequence must be non-empty when enabled = true",
            chain_label
        )));
    }

    for (i, chord) in sequence.iter().enumerate() {
        if chord.len() != n_voices {
            return Err(ConfigError::Validation(format!(
                "{}.chords.sequence[{}]: expected {} pitches, got {}",
                chain_label, i, n_voices, chord.len()
            )));
        }
        for &p in chord {
            if p > 127 {
                return Err(ConfigError::Validation(format!(
                    "{}.chords.sequence[{}]: pitch {} out of range (0..=127)", chain_label, i, p
                )));
            }
        }
    }

    Ok(ChordConfig {
        enabled: true,
        advance_on,
        advance_every: s.advance_every.unwrap_or(1).max(1),
        select,
        sequence,
    })
}

fn build_chain(
    section: &ChainSection,
    label: &str,
    shared_physics: Option<&PhysicsSection>,
) -> Result<(ChainConfig, Vec<ChannelClaim>), ConfigError> {
    let physics_section = section.physics.as_ref().or(shared_physics);
    let physics = build_physics(physics_section.cloned());
    let seed = section.seed.unwrap_or(47);

    let (midi, gate_claims) = build_midi(&section.gates, label)?;
    let (walls, wall_midi, wall_claims) = build_walls(section.walls.as_ref(), label)?;
    let clock = build_clock(&section.clock, label)?;
    let events = build_events(&section.gates, label)?;
    let modulation = build_modulation(&section.modulation, label, midi.voice_channels.first().copied())?;
    let quantize = build_quantize(&section.quantize, label)?;
    let chords = build_chords(&section.chords, midi.voice_channels.len(), label)?;

    for &site in &events.output_sites {
        if site >= physics.n_sites {
            return Err(ConfigError::Validation(format!(
                "{}.gates: voice_{} refers to a site index that doesn't exist (n_sites = {})",
                label, site, physics.n_sites
            )));
        }
    }

    let mut claims = gate_claims;
    claims.extend(wall_claims);
    claims.push(ChannelClaim {
        channel: clock.channel,
        label: format!("{}.clock", label),
    });
    if modulation.enabled {
        claims.push(ChannelClaim {
            channel: modulation.channel,
            label: format!("{}.modulation", label),
        });
    }

    let chain_cfg = ChainConfig {
        physics,
        events,
        midi,
        clock,
        walls,
        wall_midi,
        modulation,
        quantize,
        chords,
        seed,
    };

    Ok((chain_cfg, claims))
}

// --- Helpers ---------------------------------------------------------------

/// Record of "this channel is claimed by this signal in the config."
/// Used to assemble a flat list across gates, walls, and clock for the
/// duplicate-channel validator.
struct ChannelClaim {
    channel: u8,
    label: String,
}

/// Parse `voice_N` entries from a flatten map into `(index, name, entry)`
/// tuples sorted by index. Names that don't match `voice_<digits>` are
/// a validation error — we won't silently ignore typos.
fn parse_voice_indices<'a, V>(
    map: &'a BTreeMap<String, V>,
    section_label: &str,
) -> Result<Vec<(usize, &'a String, &'a V)>, ConfigError> {
    let mut out: Vec<(usize, &String, &V)> = Vec::with_capacity(map.len());
    for (name, value) in map.iter() {
        let Some(suffix) = name.strip_prefix("voice_") else {
            return Err(ConfigError::Validation(format!(
                "{}: unrecognized key '{}' (expected 'voice_N')",
                section_label, name
            )));
        };
        let idx: usize = suffix.parse().map_err(|_| {
            ConfigError::Validation(format!(
                "{}: voice key '{}' has non-numeric suffix",
                section_label, name
            ))
        })?;
        out.push((idx, name, value));
    }
    out.sort_by_key(|(idx, _, _)| *idx);
    Ok(out)
}

/// Translate a 1-based channel number from the file (1..=16) into
/// a 0-based channel for internal use. Out-of-range values are a
/// validation error.
fn validate_channel_1based(channel_1b: u8, label: &str) -> Result<u8, ConfigError> {
    if !(1..=16).contains(&channel_1b) {
        return Err(ConfigError::Validation(format!(
            "{}: channel must be in 1..=16 (got {})",
            label, channel_1b
        )));
    }
    Ok(channel_1b - 1)
}

/// Final pass: assert that no two signals across the whole config want
/// the same MIDI channel. Names both claimants in the error message,
/// matching the spec's example exactly.
fn validate_no_duplicates(claims: &[ChannelClaim]) -> Result<(), ConfigError> {
    let mut owners: HashMap<u8, String> = HashMap::new();

    for claim in claims {
        if let Some(prior) = owners.get(&claim.channel) {
            return Err(ConfigError::Validation(format!(
                "channel {} is assigned to both {} and {}",
                claim.channel + 1,
                prior,
                claim.label,
            )));
        }
        owners.insert(claim.channel, claim.label.clone());
    }

    Ok(())
}

// --- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn load_str(s: &str) -> Result<Config, ConfigError> {
        let raw: TomlConfig = toml::from_str(s).map_err(|e| ConfigError::Parse {
            path: PathBuf::from("<test>"),
            source: Box::new(e),
        })?;
        raw.into_config()
    }

    #[test]
    fn minimal_config_loads() {
        let toml = r#"
            [chain_a]
            seed = 7

            [chain_a.gates]
            voice_0 = 1
            voice_2 = 2
            voice_4 = 3
            voice_6 = 4

            [chain_a.clock]
            channel = 16
        "#;
        let config = load_str(toml).expect("should parse");
        assert_eq!(config.chain_a.seed, 7);
        assert_eq!(config.chain_a.midi.voice_channels, vec![0, 1, 2, 3]);
        assert_eq!(config.chain_a.events.output_sites, vec![0, 2, 4, 6]);
        // Clock channel was 16 (1-based) → 15 (0-based)
        assert_eq!(config.chain_a.clock.channel, 15);
        // No walls section → walls disabled, empty channel list
        assert!(!config.chain_a.walls.enabled);
        assert!(config.chain_a.wall_midi.channels.is_empty());
    }

    #[test]
    fn gate_voice_full_form_carries_pitch() {
        let toml = r#"
            [chain_a]
            [chain_a.gates]
            voice_0 = { channel = 1, pitch = 60 }
            voice_1 = 2
            [chain_a.clock]
            channel = 16
        "#;
        let config = load_str(toml).expect("should parse");
        assert_eq!(config.chain_a.midi.voice_pitches[0], 60);
        // voice_1 fell back to the default-pitch table at index 1 (E3, MIDI 52)
        assert_eq!(config.chain_a.midi.voice_pitches[1], 52);
    }

    #[test]
    fn wall_voices_become_channel_list() {
        let toml = r#"
            [chain_a]
            [chain_a.gates]
            voice_0 = 1
            [chain_a.walls]
            voice_0 = 5
            voice_1 = 6
            voice_2 = 7
            voice_3 = 8
            [chain_a.clock]
            channel = 16
        "#;
        let config = load_str(toml).expect("should parse");
        assert!(config.chain_a.walls.enabled);
        assert_eq!(config.chain_a.wall_midi.channels, vec![4, 5, 6, 7]);
    }

    #[test]
    fn duplicate_channel_is_rejected_naming_both() {
        let toml = r#"
            [chain_a]
            [chain_a.gates]
            voice_0 = 1
            [chain_a.walls]
            voice_0 = 1
            [chain_a.clock]
            channel = 16
        "#;
        let err = load_str(toml).expect_err("duplicate channel must fail");
        let msg = format!("{}", err);
        assert!(msg.contains("channel 1"));
        assert!(msg.contains("chain_a.gates.voice_0"));
        assert!(msg.contains("chain_a.walls.voice_0"));
    }

    #[test]
    fn out_of_range_channel_is_rejected() {
        let toml = r#"
            [chain_a]
            [chain_a.gates]
            voice_0 = 17
            [chain_a.clock]
            channel = 16
        "#;
        let err = load_str(toml).expect_err("channel 17 must fail");
        let msg = format!("{}", err);
        assert!(msg.contains("1..=16"));
        assert!(msg.contains("17"));
    }

    #[test]
    fn unknown_voice_key_is_rejected() {
        let toml = r#"
            [chain_a]
            [chain_a.gates]
            voixe_0 = 1
            [chain_a.clock]
            channel = 16
        "#;
        let err = load_str(toml).expect_err("typo must fail");
        let msg = format!("{}", err);
        assert!(msg.contains("voixe_0"));
    }

    #[test]
    fn no_gate_voices_is_rejected() {
        let toml = r#"
            [chain_a]
            [chain_a.gates]
            [chain_a.clock]
            channel = 16
        "#;
        let err = load_str(toml).expect_err("empty gates must fail");
        assert!(format!("{}", err).contains("at least one"));
    }

    #[test]
    fn per_chain_physics_overrides_shared() {
        let toml = r#"
            [physics]
            kt = 0.05

            [chain_a]
            [chain_a.physics]
            kt = 0.5

            [chain_a.gates]
            voice_0 = 1
            [chain_a.clock]
            channel = 16
        "#;
        let config = load_str(toml).expect("should parse");
        assert!((config.chain_a.physics.kt - 0.5).abs() < 1e-9);
    }

    #[test]
    fn shared_physics_is_used_when_per_chain_absent() {
        let toml = r#"
            [physics]
            kt = 0.42

            [chain_a]
            [chain_a.gates]
            voice_0 = 1
            [chain_a.clock]
            channel = 16
        "#;
        let config = load_str(toml).expect("should parse");
        assert!((config.chain_a.physics.kt - 0.42).abs() < 1e-9);
    }

    #[test]
    fn motion_cc_zero_disables() {
        let toml = r#"
            [chain_a]
            [chain_a.gates]
            voice_0 = 1
            [chain_a.walls]
            voice_0 = 5
            motion_cc = 0
            [chain_a.clock]
            channel = 16
        "#;
        let config = load_str(toml).expect("should parse");
        assert!(config.chain_a.wall_midi.motion_cc.is_none());
    }

    #[test]
    fn site_index_must_exist() {
        // n_sites = 4 but voice_6 references site 6
        let toml = r#"
            [physics]
            n_sites = 4

            [chain_a]
            [chain_a.gates]
            voice_0 = 1
            voice_6 = 2
            [chain_a.clock]
            channel = 16
        "#;
        let err = load_str(toml).expect_err("nonexistent site must fail");
        assert!(format!("{}", err).contains("doesn't exist"));
    }

    #[test]
    fn tempo_bpm_translates_to_drive_period() {
        let toml = r#"
            [tempo]
            bpm = 60

            [chain_a]
            [chain_a.gates]
            voice_0 = 1
            [chain_a.clock]
            channel = 16
        "#;
        let config = load_str(toml).expect("should parse");
        // 60 BPM → 1.0 sec drive period
        assert!((config.tempo.drive_period_secs - 1.0).abs() < 1e-9);
    }

    #[test]
    fn input_section_optional() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16
    "#;
        let config = load_str(toml).expect("should parse");
        assert!(config.input.is_none());
    }

    #[test]
    fn input_section_loads_with_defaults() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [input]
        [input.perturbation]
    "#;
        let config = load_str(toml).expect("should parse");
        let input = config.input.expect("input should be present");
        assert_eq!(input.perturbation.base_note, 60);
        matches!(
        input.perturbation.kind,
        crate::config::PerturbationKindConfig::Rotate { .. }
    );
    }

    #[test]
    fn input_kind_flip_parses() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [input.perturbation]
        kind = "flip"
    "#;
        let config = load_str(toml).expect("should parse");
        let input = config.input.expect("input present");
        assert!(matches!(
        input.perturbation.kind,
        crate::config::PerturbationKindConfig::Flip
    ));
    }

    #[test]
    fn input_unknown_kind_rejected() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [input.perturbation]
        kind = "wiggle"
    "#;
        let err = load_str(toml).expect_err("bad kind should fail");
        assert!(format!("{}", err).contains("wiggle"));
    }

    #[test]
    fn coupling_section_optional() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16
    "#;
        let config = load_str(toml).expect("should parse");
        assert!(config.coupling.is_none());
    }

    #[test]
    fn coupling_strength_shorthand_sets_both_directions() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [coupling]
        shape = "mean_field_z"
        strength = 0.15
    "#;
        let config = load_str(toml).expect("should parse");
        let c = config.coupling.expect("coupling present");
        assert_eq!(c.shape, crate::config::CouplingShape::MeanFieldZ);
        assert!((c.strength_ab - 0.15).abs() < 1e-9);
        assert!((c.strength_ba - 0.15).abs() < 1e-9);
    }

    #[test]
    fn coupling_asymmetric_strengths() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [coupling]
        shape = "mean_field_z"
        strength_ab = 0.2
        strength_ba = 0.05
    "#;
        let config = load_str(toml).expect("should parse");
        let c = config.coupling.expect("coupling present");
        assert!((c.strength_ab - 0.2).abs() < 1e-9);
        assert!((c.strength_ba - 0.05).abs() < 1e-9);
    }

    #[test]
    fn coupling_strength_and_split_form_are_mutually_exclusive() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [coupling]
        shape = "mean_field_z"
        strength = 0.1
        strength_ab = 0.2
    "#;
        let err = load_str(toml).expect_err("mixed forms should fail");
        assert!(format!("{}", err).contains("mutually exclusive"));
    }

    #[test]
    fn coupling_unknown_shape_rejected() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [coupling]
        shape = "telepathy"
    "#;
        let err = load_str(toml).expect_err("bad shape should fail");
        assert!(format!("{}", err).contains("telepathy"));
    }

    #[test]
    fn coupling_out_of_range_strength_rejected() {
        let toml = r#"
        [chain_a]
        [chain_a.gates]
        voice_0 = 1
        [chain_a.clock]
        channel = 16

        [coupling]
        shape = "mean_field_z"
        strength = 5.0
    "#;
        let err = load_str(toml).expect_err("out of range should fail");
        assert!(format!("{}", err).contains("0.0..=2.0"));
    }

    #[test]
    fn coupling_site_paired_and_shared_drive_parse() {
        // The two stubs should parse cleanly even though the runtime
        // doesn't implement them yet. The config layer's job is just
        // typed validation.
        for shape in ["site_paired", "shared_drive"] {
            let toml = format!(
                r#"
            [chain_a]
            [chain_a.gates]
            voice_0 = 1
            [chain_a.clock]
            channel = 16

            [coupling]
            shape = "{}"
            strength = 0.1
            "#,
                shape
            );
            load_str(&toml).unwrap_or_else(|_| panic!("{} should parse", shape));
        }
    }
}
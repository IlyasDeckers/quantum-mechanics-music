//! Configuration for the substrate, output, and OSC layers.

mod chords;
mod clock;
pub mod config_file;
mod coupling;
mod events;
mod midi;
mod modulation;
mod osc;
mod physics;
mod quantize;
mod tempo;
mod walls;
mod input;

pub use chords::{ChordConfig, ChordSelect, ChordTriggerKind};
pub use clock::ClockConfig;
pub use coupling::{CouplingConfig, CouplingShape};
pub use events::EventConfig;
pub use input::{InputConfig, PerturbationConfig, PerturbationKindConfig};
pub use midi::MidiConfig;
pub use modulation::ModulationConfig;
pub use osc::OscConfig;
pub use physics::{
    apply_smoothing, apply_smoothing_to_f64,
    PhysicsConfig, PhysicsTargets, SmoothingAlphas, SmoothingConfig,
};
pub use quantize::QuantizeConfig;
pub use tempo::TempoConfig;
pub use walls::{WallConfig, WallMidiConfig};

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub chain_a: ChainConfig,
    pub chain_b: Option<ChainConfig>,
    pub coupling: Option<CouplingConfig>,
    pub tempo: TempoConfig,
    pub osc: OscConfig,
    pub input: Option<InputConfig>,
}

#[derive(Clone, Debug)]
pub struct ChainConfig {
    pub physics: PhysicsConfig,
    pub events: EventConfig,
    pub midi: MidiConfig,
    pub clock: ClockConfig,
    pub walls: WallConfig,
    pub wall_midi: WallMidiConfig,
    pub modulation: ModulationConfig,
    pub quantize: QuantizeConfig,
    pub chords: ChordConfig,
    pub seed: u64,
    /// Emit gate/pulse/modulation/wall events every Nth physics tick.
    /// Decouples emission cadence from physics resolution: keep
    /// `ticks_per_period` high for rich inter-kick dynamics, raise
    /// `emit_stride` to thin the output grid. 1 = emit every tick.
    pub emit_stride: u32,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            physics: PhysicsConfig::default(),
            events: EventConfig::default(),
            midi: MidiConfig::default(),
            clock: ClockConfig::default(),
            walls: WallConfig::default(),
            wall_midi: WallMidiConfig::default(),
            modulation: ModulationConfig::default(),
            quantize: QuantizeConfig::default(),
            chords: ChordConfig::default(),
            seed: 0,
            emit_stride: 1,
        }
    }
}
//! Chord-change configuration.
//!
//! A `ChordEngine` rewrites the shared `voice_pitches` arc that
//! `EventDetector` reads at gate time. Each chain owns its own
//! engine and its own sequence of voicings.

#[derive(Clone, Debug)]
pub struct ChordConfig {
    pub enabled: bool,
    /// Which physics event advances the chord pointer.
    pub advance_on: ChordTriggerKind,
    /// How many of those events must fire before advancing.
    pub advance_every: u32,
    /// How the next chord index is chosen.
    pub select: ChordSelect,
    /// The chord library. Each inner Vec is one voicing, length must
    /// match the number of gate voices on this chain.
    pub sequence: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordTriggerKind {
    Clock,      // chain clock pulse
    WallDeath,  // a domain wall is destroyed
    Period,     // every N drive periods (counted by ticks_per_period)
}

#[derive(Clone, Copy, Debug)]
pub enum ChordSelect {
    Sequential,    // round-robin through sequence
    Magnetization, // index = floor(normalized_mag * n_chords)
    Random,        // seeded from tick so it's reproducible on same seed
}

impl Default for ChordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            advance_on: ChordTriggerKind::WallDeath,
            advance_every: 1,
            select: ChordSelect::Sequential,
            sequence: Vec::new(),
        }
    }
}

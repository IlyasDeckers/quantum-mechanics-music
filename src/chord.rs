//! Chord-change engine.
//!
//! Rewrites the shared `voice_pitches` arc that `EventDetector` reads
//! at gate time. Each chain owns its own engine. Advances on a chosen
//! physics event (clock pulse, wall destruction, or drive period
//! boundary), and the next chord is picked sequentially, by chain
//! magnetization, or by a tick-seeded RNG.

use crate::config::{ChordConfig, ChordSelect, ChordTriggerKind};
use std::sync::{Arc, RwLock};

pub enum ChordTrigger {
    Clock,
    WallDeath,
    Period,
}

pub struct ChordEngine {
    config: ChordConfig,
    current_idx: usize,
    counter: u32,
    /// Shared with EventDetector. Writing here takes effect on the next
    /// gate event from any voice in this chain.
    voice_pitches: Arc<RwLock<Vec<u8>>>,
}

impl ChordEngine {
    pub fn new(config: ChordConfig, voice_pitches: Arc<RwLock<Vec<u8>>>) -> Option<Self> {
        if !config.enabled || config.sequence.is_empty() {
            return None;
        }
        let engine = Self { config, current_idx: 0, counter: 0, voice_pitches };
        engine.apply_chord();
        Some(engine)
    }

    /// Call once per relevant event. `magnetization` is only used when
    /// `select = Magnetization`; pass `chain.global_magnetization()`.
    pub fn on_event(&mut self, trigger: ChordTrigger, magnetization: f64, tick: u64) {
        let matches = matches!(
            (&trigger, &self.config.advance_on),
            (ChordTrigger::Clock,     ChordTriggerKind::Clock)     |
            (ChordTrigger::WallDeath, ChordTriggerKind::WallDeath) |
            (ChordTrigger::Period,    ChordTriggerKind::Period)
        );
        if !matches { return; }

        self.counter += 1;
        if self.counter < self.config.advance_every {
            return;
        }
        self.counter = 0;

        let n = self.config.sequence.len();
        let next = match self.config.select {
            ChordSelect::Sequential => (self.current_idx + 1) % n,
            ChordSelect::Magnetization => {
                let norm = ((magnetization + 1.0) / 2.0).clamp(0.0, 1.0);
                let idx = (norm * n as f64).floor() as usize;
                idx.min(n - 1)
            }
            ChordSelect::Random => {
                ((tick.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
                    >> 33) as usize) % n
            }
        };

        if next == self.current_idx { return; }
        self.current_idx = next;
        self.apply_chord();
    }

    fn apply_chord(&self) {
        let chord = &self.config.sequence[self.current_idx];
        if let Ok(mut pitches) = self.voice_pitches.write() {
            for (i, &p) in chord.iter().enumerate() {
                if let Some(slot) = pitches.get_mut(i) {
                    *slot = p;
                }
            }
        }
    }
}

//! One chain and its per-chain plumbing: detectors, emitters,
//! physics targets, RNG, optional MIDI input. Runtime owns a Vec
//! of these; coupling and multi-chain orchestration live in Runtime.

use crate::chord::{ChordEngine, ChordTrigger};
use crate::clock::ClockEmitter;
use crate::events::EventDetector;
use crate::input::MidiInputListener;
use crate::midi::MidiSender;
use crate::modulation::ModulationEmitter;
use crate::osc::OutboundEvent;
use crate::osc_io::OscSink;
use crate::perturbation::PerturbationRouter;
use crate::quantizer::ScaleQuantizer;
use crate::wall_midi::WallVoiceAllocator;
use crate::walls::WallDetector;
use crate::tui::{LogSource, TuiState};
use arc_swap::ArcSwap;
use crystallized_time::chain::SpinChain;
use crystallized_time::chain_id::ChainId;
use crystallized_time::config::{
    apply_smoothing, ChordConfig, PhysicsConfig, PhysicsTargets, SmoothingAlphas,
};
use rand::rngs::StdRng;
use std::sync::{Arc, RwLock};

use crystallized_time::config::ModulationConfig;

pub struct ChainPipeline {
    pub id: ChainId,
    pub chain: SpinChain,
    pub physics_arc: Arc<ArcSwap<PhysicsConfig>>,
    pub targets: Arc<RwLock<PhysicsTargets>>,

    detector: EventDetector,
    clock_emitter: ClockEmitter,
    wall_detector: WallDetector,
    wall_voicer: WallVoiceAllocator,
    modulation_emitter: ModulationEmitter,
    rng: StdRng,

    input_listener: Option<MidiInputListener>,
    perturbation_router: Option<PerturbationRouter>,

    quantizer: Arc<RwLock<Option<ScaleQuantizer>>>,

    chord_engine: Option<ChordEngine>,

    n_sites: usize,
    emit_stride: u32,
    ticks_per_period: u32,
}

impl ChainPipeline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ChainId,
        physics: PhysicsConfig,
        events_cfg: crystallized_time::config::EventConfig,
        midi_cfg: crystallized_time::config::MidiConfig,
        clock_cfg: crystallized_time::config::ClockConfig,
        walls_cfg: crystallized_time::config::WallConfig,
        wall_midi_cfg: crystallized_time::config::WallMidiConfig,
        modulation_cfg: ModulationConfig,
        chord_cfg: ChordConfig,
        seed: u64,
        emit_stride: u32,
        targets: Arc<RwLock<PhysicsTargets>>,
        input_listener: Option<MidiInputListener>,
        perturbation_router: Option<PerturbationRouter>,
        voice_pitch_overrides: Option<Arc<RwLock<Vec<u8>>>>,
        quantizer: Arc<RwLock<Option<ScaleQuantizer>>>,
    ) -> Self {
        use rand::SeedableRng;

        let mut rng = StdRng::seed_from_u64(seed);
        let physics_arc = Arc::new(ArcSwap::from_pointee(physics.clone()));
        let chain = SpinChain::new(Arc::clone(&physics_arc), &mut rng);

        let detector = EventDetector::new(events_cfg.clone(), midi_cfg, &chain, voice_pitch_overrides.as_ref().map(Arc::clone));
        let clock_emitter = ClockEmitter::new(clock_cfg, &chain, id, Arc::clone(&quantizer));
        let wall_detector = WallDetector::new(walls_cfg);
        let wall_voicer = WallVoiceAllocator::new(wall_midi_cfg, &physics, id, Arc::clone(&quantizer));
        let modulation_emitter =
            ModulationEmitter::new(modulation_cfg, events_cfg.output_sites);

        let chord_engine = voice_pitch_overrides
            .as_ref()
            .and_then(|vp| ChordEngine::new(chord_cfg, Arc::clone(vp)));

        Self {
            id,
            chain,
            physics_arc,
            targets,
            detector,
            clock_emitter,
            wall_detector,
            wall_voicer,
            modulation_emitter,
            rng,
            input_listener,
            perturbation_router,
            quantizer,
            chord_engine,
            n_sites: physics.n_sites,
            emit_stride,
            ticks_per_period: physics.ticks_per_period,
        }
    }

    pub fn should_emit(&self, tick: u64) -> bool {
        self.emit_stride <= 1 || tick.is_multiple_of(self.emit_stride as u64)
    }

    pub fn advance_smoothing(&self, alphas: &SmoothingAlphas) {
        let current = self.physics_arc.load();
        let targets_snapshot = match self.targets.read() {
            Ok(t) => t.clone(),
            Err(_) => {
                eprintln!(
                    "warning: {} physics targets lock poisoned, skipping smoothing",
                    self.id.label()
                );
                return;
            }
        };
        if let Some(new_cfg) = apply_smoothing(&current, &targets_snapshot, alphas) {
            drop(current);
            self.physics_arc.store(Arc::new(new_cfg));
        }
    }

    pub fn step_physics(&mut self) {
        self.chain.step(&mut self.rng);
    }

    pub fn apply_input_perturbations(&mut self, tui: Option<&TuiState>) {
        let (Some(listener), Some(router)) = (
            self.input_listener.as_ref(),
            self.perturbation_router.as_ref(),
        ) else {
            return;
        };

        for msg in listener.poll() {
            if let Some((site, kind)) = router.route(msg, self.n_sites) {
                self.chain.perturb(site, kind);
            }
            if let Some(tui) = tui {
                let ch = (msg.status & 0x0F) + 1;
                let content = format!("ch{} n{} v{}", ch, msg.data1, msg.data2);
                tui.push_log(LogSource::Midi, content);
            }
        }
    }

    pub fn emit_site_events(
        &mut self,
        midi: &MidiSender,
        mut osc: Option<&mut OscSink>,
    ) -> usize {
        let events = self.detector.check(&self.chain);
        let count = events.len();
        let quant = self.quantizer.read().unwrap();
        for mut event in events {
            if let Some(ref q) = *quant {
                event.pitch = q.quantize(event.pitch);
            }
            midi.send_gate(event);
            if let Some(sink) = osc.as_deref_mut() {
                sink.push(OutboundEvent::SiteEvent {
                    chain: self.id,
                    site: event.site as u8,
                    voice: event.voice,
                    intensity: event.intensity,
                });
            }
        }
        count
    }

    pub fn tick_clock(
        &mut self,
        midi: &MidiSender,
        osc: Option<&mut OscSink>,
    ) -> bool {
        self.clock_emitter.tick(&self.chain, midi, osc)
    }

    pub fn tick_modulation(&mut self, midi: &MidiSender) {
        self.modulation_emitter.tick(&self.chain, midi);
    }

    pub fn tick_chords(
        &mut self,
        clock_pulsed: bool,
        walls_destroyed: usize,
        is_period_boundary: bool,
    ) {
        let engine = match self.chord_engine.as_mut() {
            Some(e) => e,
            None => return,
        };
        let mag = self.chain.global_magnetization();
        let tick = self.chain.tick;

        if clock_pulsed {
            engine.on_event(ChordTrigger::Clock, mag, tick);
        }
        for _ in 0..walls_destroyed {
            engine.on_event(ChordTrigger::WallDeath, mag, tick);
        }
        if is_period_boundary {
            engine.on_event(ChordTrigger::Period, mag, tick);
        }
    }

    pub fn ticks_per_period(&self) -> u32 {
        self.ticks_per_period
    }

    pub fn process_walls(
        &mut self,
        midi: &MidiSender,
        mut osc: Option<&mut OscSink>,
    ) -> (usize, usize, usize) {
        let wall_events = self.wall_detector.check(&self.chain);
        let mut created = 0usize;
        let mut moved = 0usize;
        let mut destroyed = 0usize;
        for event in &wall_events {
            use crate::walls::WallEvent;
            match event {
                WallEvent::Created { .. } => created += 1,
                WallEvent::Moved { .. } => moved += 1,
                WallEvent::Destroyed { .. } => destroyed += 1,
            }
            self.wall_voicer.handle(event, midi, osc.as_deref_mut());
        }
        (created, moved, destroyed)
    }

    pub fn push_state(&self, sink: &mut OscSink) {
        let spins: Vec<f64> = self.chain.spins.iter().map(|s| s[2]).collect();
        sink.push_state_if_due(
            self.id,
            &spins,
            self.chain.global_magnetization(),
            self.wall_detector.wall_count(),
        );
    }

    pub fn get_magnetization(&self) -> f64 {
        self.chain.global_magnetization()
    }

    pub fn get_wall_count(&self) -> usize {
        self.wall_detector.wall_count()
    }

    pub fn get_physics_config(&self) -> PhysicsConfig {
        (*self.physics_arc.load_full()).clone()
    }
}
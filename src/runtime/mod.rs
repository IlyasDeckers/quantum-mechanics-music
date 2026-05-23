use crate::input::MidiInputListener;
use crate::midi::MidiSender;
use crate::osc_io::OscSink;
use crate::perturbation::PerturbationRouter;
use crate::quantizer::ScaleQuantizer;
use crate::tui::{CouplingInfo, LogSource, TuiState};
use crystallized_time::chain_id::ChainId;
use crystallized_time::config::{
    Config, PhysicsTargets, SmoothingAlphas, SmoothingConfig,
};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

mod pipeline;
mod coupling;

use pipeline::ChainPipeline;
pub use coupling::{CouplingState, CouplingTargets};

pub struct Runtime {
    pipelines: Vec<ChainPipeline>,
    coupling: Option<CouplingState>,
    midi_sender: MidiSender,
    osc_sink: Option<OscSink>,
    alphas: SmoothingAlphas,
    tick_duration: Duration,
    gate_length_ms: u64,
    tui_state: Option<Arc<TuiState>>,
}

impl Runtime {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        config: &Config,
        midi_sender: MidiSender,
        osc_sink: Option<OscSink>,
        targets_a: Arc<RwLock<PhysicsTargets>>,
        targets_b: Option<Arc<RwLock<PhysicsTargets>>>,
        coupling_targets: Option<Arc<RwLock<CouplingTargets>>>,
        input_listener: Option<MidiInputListener>,
        perturbation_router: Option<PerturbationRouter>,
        tui_state: Option<Arc<TuiState>>,
        voice_pitches_a: Option<Arc<RwLock<Vec<u8>>>>,
        voice_pitches_b: Option<Arc<RwLock<Vec<u8>>>>,
        quantizer_a: Arc<RwLock<Option<ScaleQuantizer>>>,
        quantizer_b: Option<Arc<RwLock<Option<ScaleQuantizer>>>>,
    ) -> Self {
        let dt_real_secs =
            config.tempo.drive_period_secs / config.chain_a.physics.ticks_per_period as f64;
        let alphas = SmoothingAlphas::from_config(&SmoothingConfig::default(), dt_real_secs);
        let tick_duration = Duration::from_secs_f64(dt_real_secs);
        let mut pipelines = Vec::with_capacity(2);

        let chain_a = ChainPipeline::new(
            ChainId::A,
            config.chain_a.physics.clone(),
            config.chain_a.events.clone(),
            config.chain_a.midi.clone(),
            config.chain_a.clock.clone(),
            config.chain_a.walls.clone(),
            config.chain_a.wall_midi.clone(),
            config.chain_a.modulation.clone(),
            config.chain_a.seed,
            config.chain_a.emit_stride,
            targets_a,
            input_listener,
            perturbation_router,
            voice_pitches_a,
            quantizer_a,
        );
        pipelines.push(chain_a);

        if let Some(b_cfg) = &config.chain_b {
            let targets_b = targets_b.expect(
                "chain_b is configured but no PhysicsTargets was provided"
            );
            let chain_b = ChainPipeline::new(
                ChainId::B,
                b_cfg.physics.clone(),
                b_cfg.events.clone(),
                b_cfg.midi.clone(),
                b_cfg.clock.clone(),
                b_cfg.walls.clone(),
                b_cfg.wall_midi.clone(),
                b_cfg.modulation.clone(),
                b_cfg.seed,
                b_cfg.emit_stride,
                targets_b,
                None,
                None,
                voice_pitches_b,
                quantizer_b.expect("chain_b is configured but no quantizer was provided"),
            );
            pipelines.push(chain_b);
        }

        let coupling = match (&config.coupling, &config.chain_b, coupling_targets) {
            (Some(c), Some(_), Some(targets)) => Some(CouplingState::new_with_targets(c, targets)),
            (Some(_), None, _) => {
                eprintln!(
                    "warning: [coupling] section is present in the config but chain_b is \
                     absent; coupling will not run"
                );
                None
            }
            _ => None,
        };

        Self {
            pipelines,
            coupling,
            midi_sender,
            osc_sink,
            alphas,
            tick_duration,
            gate_length_ms: config.chain_a.midi.gate_length_ms,
            tui_state,
        }
    }

    pub fn run_until(
        &mut self,
        total_ticks: u64,
        running: &std::sync::atomic::AtomicBool,
    ) {
        use std::sync::atomic::Ordering;

        let start = Instant::now();
        for tick in 1..=total_ticks {
            if !running.load(Ordering::Acquire) {
                break;
            }

            self.step(tick);

            let target = start + self.tick_duration * tick as u32;
            let now = Instant::now();
            if target > now {
                std::thread::sleep(target - now);
            }
        }
    }

    fn step(&mut self, tick: u64) {
        // Phase 1: smoothing. Each pipeline independent.
        for p in &self.pipelines {
            p.advance_smoothing(&self.alphas);
        }

        // Phase 1.5: coupling. Snapshot both chains' state, then inject
        // coupling fields. Must happen between smoothing (so coupling sees
        // smoothed parameters) and physics (so injected deltas land before
        // the chain consumes them in step_physics).
        if let Some(coupling) = self.coupling.as_mut() {
            coupling.advance_smoothing(&self.alphas);
            let snapshot = coupling.snapshot(&self.pipelines);
            coupling.inject(&snapshot, &mut self.pipelines);
        }

        // Phase 2: physics.
        for p in &mut self.pipelines {
            p.step_physics();
        }

        // Phase 3: input. Each pipeline drains its own MIDI input.
        for p in &mut self.pipelines {
            p.apply_input_perturbations(self.tui_state.as_deref());
        }

        // Phase 4: emit. Site events, clock, modulation CC, walls. Per-pipeline.
        // Pipelines with emit_stride > 1 skip emission on most ticks, so we
        // can keep rich physics resolution and a sparse output grid at the
        // same time.
        for p in &mut self.pipelines {
            if !p.should_emit(tick) {
                continue;
            }
            let gate_count = p.emit_site_events(&self.midi_sender, self.osc_sink.as_mut());
            let pulsed = p.tick_clock(&self.midi_sender, self.osc_sink.as_mut());
            p.tick_modulation(&self.midi_sender);
            let (created, _moved, destroyed) =
                p.process_walls(&self.midi_sender, self.osc_sink.as_mut());

            if let Some(tui) = self.tui_state.as_deref() {
                let label = p.id.osc_prefix().trim_start_matches('/');
                for i in 0..gate_count {
                    tui.push_log(LogSource::Internal, format!("GATE {} #{}", label, i));
                }
                if pulsed {
                    tui.push_log(LogSource::Internal, format!("CLOCK {}", label));
                }
                for _ in 0..created {
                    tui.push_log(LogSource::Internal, format!("WALL {} new", label));
                }
                for _ in 0..destroyed {
                    tui.push_log(LogSource::Internal, format!("WALL {} destroyed", label));
                }
            }
        }

        // Phase 5: state push + flush.
        if let Some(sink) = self.osc_sink.as_mut() {
            for p in &self.pipelines {
                p.push_state(sink);
            }
            sink.flush_tick();
        }

        // Phase 6: TUI state update.
        self.update_tui_state(tick);
    }

    fn update_tui_state(&self, tick: u64) {
        let tui = match self.tui_state.as_deref() {
            Some(t) => t,
            None => return,
        };

        tui.tick.store(tick, std::sync::atomic::Ordering::Relaxed);

        // Update coupling info.
        if let Some(ref coupling) = self.coupling {
            if let Ok(mut tui_coupling) = tui.coupling.write() {
                *tui_coupling = Some(CouplingInfo {
                    shape: coupling.shape_string(),
                    strength_ab: coupling.current_ab(),
                    strength_ba: coupling.current_ba(),
                });
            }
        }

        // Update per-chain state.
        for (i, p) in self.pipelines.iter().enumerate() {
            let chain_state = &tui.chains[i];
            if !chain_state.present {
                continue;
            }

            let m = p.get_magnetization();
            let wc = p.get_wall_count();

            let _ = chain_state.magnetization.write().map(|mut g| *g = m);
            let _ = chain_state.wall_count.write().map(|mut g| *g = wc as u64);

            // Push magnetization to scope ring buffer.
            if let Ok(mut bufs) = tui.scope_bufs.write() {
                let buf = &mut bufs[i];
                if buf.len() >= tui.scope_buf_cap {
                    buf.pop_front();
                }
                buf.push_back(m);
            }

            // Read physics parameters from ArcSwap.
            let physics = p.get_physics_config();
            let _ = chain_state.kt.write().map(|mut g| *g = physics.kt);
            let _ = chain_state.eps.write().map(|mut g| *g = physics.eps);
            let _ = chain_state.j.write().map(|mut g| *g = physics.j);
            let _ = chain_state.w.write().map(|mut g| *g = physics.w);
        }
    }

    pub fn shutdown(self) {
        self.midi_sender.shutdown();
        std::thread::sleep(Duration::from_millis(self.gate_length_ms + 50));
    }
}
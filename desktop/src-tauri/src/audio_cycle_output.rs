//! 普通播放路径唯一的 PortAudio PCM 生产者。
//!
//! FFmpeg 任务只填充各自的有界 PCM 缓冲。本模块按值拥有 `PortAudioOutput`，
//! 所有播放、交叉淡化和测试音都通过同一个控制队列进入同一生产线程。

use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio_mixer::{
    AudioMixerTrack, AUDIO_CANDIDATE_COMMIT_TAIL_MS, AUDIO_CROSSFADE_MS,
    AUDIO_POST_CROSSFADE_TAIL_MS,
};
use crate::audio_output_diagnostic::{
    AudioLowFrequencyDiagnosticSnapshot, LowFrequencyDiagnosticAnalyzer,
};

const OUTPUT_CHANNELS: usize = 2;
const DIGITAL_SILENCE_PEAK: f32 = 0.000_000_1;
const AUDIBLE_REFERENCE_PEAK: f32 = 0.001;
pub const AUDIO_CANDIDATE_DIGITAL_SILENCE_ERROR: &str = "候选音轨首段为数字静音，保持当前音轨";
type CrossfadePair = (Vec<f32>, Vec<f32>);
const OUTPUT_CHUNK_FRAMES: usize = 512;
const CONTROL_TIMEOUT: Duration = Duration::from_millis(1_000);
const STARTUP_TIMEOUT: Duration = Duration::from_millis(5_000);
const CROSSFADE_WAIT_TIMEOUT: Duration = Duration::from_millis(500);
const RING_CLEAR_TIMEOUT: Duration = Duration::from_millis(500);
const STATUS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
pub const AUDIO_CYCLE_CROSSFADE_MS: usize = AUDIO_CROSSFADE_MS;

type ControlResponse = mpsc::Sender<Result<(), String>>;

#[derive(Debug, Clone, Copy)]
pub struct AudioOutputConfig {
    pub device_index: Option<i32>,
    pub sample_rate_hz: u32,
    pub memory_buffer_kib: u32,
    pub frames_per_buffer: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioCycleOutputStatus {
    pub health: autolive_portaudio_output::PortAudioStreamHealth,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub device_index: Option<i32>,
    pub memory_buffer_kib: u32,
    pub frames_per_buffer: u32,
    /// 由采样率和硬件 callback 帧数计算的有效播放水位，不等于内存容量。
    pub playback_watermark_ms: u64,
    pub callback_paused: bool,
    /// 依据 callback 已消费 frame 和实际硬件延迟估算的当前可听媒体绝对时间。
    pub timeline_media_position_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioTrackTimeline {
    pub media_position_ms: u64,
    pub playback_rate: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioTestTone {
    pub frequency_hz: f32,
    pub duration_ms: u32,
    pub amplitude: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct AudioInterludeMixConfig {
    pub volume_gain: f32,
    pub duck_gain: f32,
    pub attack_ms: u64,
    pub release_ms: u64,
}

#[derive(Debug)]
enum OutputCommand {
    SetMediaGain {
        gain: f32,
        response: ControlResponse,
    },
    SetCurrent {
        track: AudioMixerTrack,
        timeline: AudioTrackTimeline,
        response: ControlResponse,
    },
    CrossfadeTo {
        track: AudioMixerTrack,
        timeline: AudioTrackTimeline,
        response: ControlResponse,
    },
    PlayTestTone {
        tone: AudioTestTone,
        response: ControlResponse,
    },
    StartInterlude {
        track: AudioMixerTrack,
        config: AudioInterludeMixConfig,
        response: ControlResponse,
    },
    CrossfadeInterludeTo {
        track: AudioMixerTrack,
        response: ControlResponse,
    },
    SetInterludeGain {
        gain: f32,
        response: ControlResponse,
    },
    PauseInterlude(ControlResponse),
    ResumeInterlude(ControlResponse),
    StopInterlude(ControlResponse),
    Pause(ControlResponse),
    Resume(ControlResponse),
    Clear(ControlResponse),
    Stop(ControlResponse),
}

#[derive(Debug)]
struct PendingCrossfade {
    next: AudioMixerTrack,
    timeline: AudioTrackTimeline,
    response: ControlResponse,
    deadline: Instant,
}

#[derive(Debug)]
struct ToneState {
    skipped_track_samples: usize,
    restore_paused: bool,
}

#[derive(Debug)]
enum InterludeEnvelope {
    Attack { frame: usize, total_frames: usize },
    Sustain,
    Release { frame: usize, total_frames: usize },
}

#[derive(Debug)]
struct InterludeMixState {
    track: AudioMixerTrack,
    pending_crossfade: Option<PendingInterludeCrossfade>,
    volume_gain: f32,
    duck_gain: f32,
    attack_frames: usize,
    release_frames: usize,
    envelope: InterludeEnvelope,
    paused: bool,
}

#[derive(Debug)]
struct PendingInterludeCrossfade {
    next: AudioMixerTrack,
    response: ControlResponse,
    completed_frames: usize,
    total_frames: usize,
    deadline: Instant,
}

impl InterludeMixState {
    fn new(track: AudioMixerTrack, config: AudioInterludeMixConfig, sample_rate_hz: u32) -> Self {
        let attack_frames = frames_for_ms(sample_rate_hz, config.attack_ms);
        let envelope = if attack_frames == 0 {
            InterludeEnvelope::Sustain
        } else {
            InterludeEnvelope::Attack {
                frame: 0,
                total_frames: attack_frames,
            }
        };
        Self {
            track,
            pending_crossfade: None,
            volume_gain: sanitize_gain(config.volume_gain, 4.0),
            duck_gain: sanitize_gain(config.duck_gain, 1.0),
            attack_frames,
            release_frames: frames_for_ms(sample_rate_hz, config.release_ms),
            envelope,
            paused: false,
        }
    }

    fn pause(&mut self) {
        self.cancel_pending_crossfade("PortAudio 插话已暂停");
        self.paused = true;
    }

    fn resume(&mut self) {
        self.paused = false;
        self.envelope = if self.attack_frames == 0 {
            InterludeEnvelope::Sustain
        } else {
            InterludeEnvelope::Attack {
                frame: 0,
                total_frames: self.attack_frames,
            }
        };
    }

    fn begin_release(&mut self) {
        self.cancel_pending_crossfade("PortAudio 插话已停止");
        self.paused = false;
        self.envelope = InterludeEnvelope::Release {
            frame: 0,
            total_frames: self.release_frames,
        };
    }

    fn set_volume_gain(&mut self, gain: f32) {
        self.volume_gain = sanitize_gain(gain, 4.0);
    }

    fn start_crossfade(
        &mut self,
        next: AudioMixerTrack,
        sample_rate_hz: u32,
        response: ControlResponse,
    ) {
        let error = if self.paused {
            Some("PortAudio 插话已暂停，不能切换声音预设".to_owned())
        } else if self.pending_crossfade.is_some() {
            Some("PortAudio 插话声音预设正在切换".to_owned())
        } else {
            None
        };
        if let Some(error) = error {
            let _ = response.send(Err(error));
            return;
        }
        let total_samples = stereo_samples_for_ms(sample_rate_hz, AUDIO_CYCLE_CROSSFADE_MS);
        let minimum_candidate_samples =
            stereo_samples_for_ms(sample_rate_hz, AUDIO_CANDIDATE_COMMIT_TAIL_MS);
        if let Err(error) = validate_candidate_signal(&self.track, &next, minimum_candidate_samples)
        {
            let _ = response.send(Err(error));
            return;
        }
        if self.track.available_samples() < total_samples {
            let _ = response.send(Err("当前插话 PCM 不足以执行平滑预设切换".to_owned()));
            return;
        }
        self.pending_crossfade = Some(PendingInterludeCrossfade {
            next,
            response,
            completed_frames: 0,
            total_frames: total_samples / OUTPUT_CHANNELS,
            deadline: Instant::now() + CROSSFADE_WAIT_TIMEOUT,
        });
    }

    fn cancel_pending_crossfade(&mut self, reason: &str) {
        if let Some(pending) = self.pending_crossfade.take() {
            let _ = pending.response.send(Err(reason.to_owned()));
        }
    }

    fn take_interlude_samples(&mut self, sample_count: usize) -> Result<Vec<f32>, String> {
        let sample_count = sample_count - sample_count % OUTPUT_CHANNELS;
        let Some(mut pending) = self.pending_crossfade.take() else {
            return self.track.take_up_to(sample_count);
        };
        let remaining_frames = pending
            .total_frames
            .saturating_sub(pending.completed_frames);
        let fade_samples = sample_count.min(remaining_frames.saturating_mul(OUTPUT_CHANNELS));
        if fade_samples == 0 {
            self.track = pending.next;
            let _ = pending.response.send(Ok(()));
            return self.track.take_up_to(sample_count);
        }
        if self.track.available_samples() < fade_samples
            || pending.next.available_samples() < fade_samples
        {
            if Instant::now() >= pending.deadline {
                let _ = pending
                    .response
                    .send(Err("插话候选未在 500ms 内提供完整交叉淡化 PCM".to_owned()));
            } else {
                self.pending_crossfade = Some(pending);
            }
            return self.track.take_up_to(sample_count);
        }
        let old = self
            .track
            .take_exact(fade_samples)?
            .ok_or_else(|| "当前插话 PCM 在交叉淡化期间意外不足".to_owned())?;
        let next = pending
            .next
            .take_exact(fade_samples)?
            .ok_or_else(|| "候选插话 PCM 在交叉淡化期间意外不足".to_owned())?;
        let mut mixed =
            linear_crossfade_window(&old, &next, pending.completed_frames, pending.total_frames);
        pending.completed_frames = pending
            .completed_frames
            .saturating_add(fade_samples / OUTPUT_CHANNELS);
        if pending.completed_frames >= pending.total_frames {
            self.track = pending.next;
            let _ = pending.response.send(Ok(()));
            mixed.extend(
                self.track
                    .take_up_to(sample_count.saturating_sub(fade_samples))?,
            );
        } else {
            self.pending_crossfade = Some(pending);
        }
        Ok(mixed)
    }

    fn envelope_gain(&mut self) -> (f32, bool) {
        match &mut self.envelope {
            InterludeEnvelope::Attack {
                frame,
                total_frames,
            } => {
                *frame = frame.saturating_add(1);
                let gain = (*frame as f32 / (*total_frames).max(1) as f32).clamp(0.0, 1.0);
                if *frame >= *total_frames {
                    self.envelope = InterludeEnvelope::Sustain;
                }
                (gain, true)
            }
            InterludeEnvelope::Sustain => (1.0, true),
            InterludeEnvelope::Release {
                frame,
                total_frames,
            } => {
                if *total_frames == 0 {
                    return (0.0, false);
                }
                *frame = frame.saturating_add(1);
                let gain = (1.0 - *frame as f32 / *total_frames as f32).clamp(0.0, 1.0);
                (gain, *frame < *total_frames)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TimelineAnchor {
    callback_frame: u64,
    media_position_ms: u64,
    playback_rate: f64,
}

#[derive(Debug, Default)]
struct OutputTimeline {
    active: Option<TimelineAnchor>,
    pending: Option<TimelineAnchor>,
}

impl OutputTimeline {
    fn media_position_ms(
        &mut self,
        output: &autolive_portaudio_output::PortAudioOutput,
    ) -> Option<u64> {
        let health = output.stream_health();
        let sample_rate_hz = health
            .actual_sample_rate_hz
            .unwrap_or_else(|| output.sample_rate_hz())
            .max(1);
        let latency_us = health
            .output_latency_us
            .unwrap_or(0)
            .max(u64::try_from(health.callback_output_buffer_dac_time_delta_us).unwrap_or(0));
        let latency_frames = latency_us
            .saturating_mul(u64::from(sample_rate_hz))
            .saturating_add(999_999)
            .saturating_div(1_000_000);
        let audible_callback_frame = health
            .callback_pcm_frames_total
            .saturating_sub(latency_frames);
        if self
            .pending
            .is_some_and(|anchor| audible_callback_frame >= anchor.callback_frame)
        {
            self.active = self.pending.take();
        }
        self.active.map(|anchor| {
            estimate_media_position_ms(anchor, audible_callback_frame, sample_rate_hz)
        })
    }
}

fn normalized_playback_rate(playback_rate: f64) -> f64 {
    if playback_rate.is_finite() && playback_rate > 0.0 {
        playback_rate
    } else {
        1.0
    }
}

fn estimate_media_position_ms(
    anchor: TimelineAnchor,
    audible_callback_frame: u64,
    sample_rate_hz: u32,
) -> u64 {
    let elapsed_frames = audible_callback_frame.saturating_sub(anchor.callback_frame);
    let elapsed_media_ms = (elapsed_frames as f64 * 1_000.0 * anchor.playback_rate
        / f64::from(sample_rate_hz.max(1)))
    .round()
    .clamp(0.0, u64::MAX as f64) as u64;
    anchor.media_position_ms.saturating_add(elapsed_media_ms)
}

#[derive(Debug, Clone)]
pub struct AudioCycleOutputControl {
    commands: mpsc::SyncSender<OutputCommand>,
    status: Arc<Mutex<AudioCycleOutputStatus>>,
    diagnostic: Arc<Mutex<AudioLowFrequencyDiagnosticSnapshot>>,
    failure: Arc<Mutex<Option<String>>>,
}

#[derive(Debug)]
pub struct AudioCycleOutputTask {
    control: AudioCycleOutputControl,
    handle: Option<JoinHandle<()>>,
}

impl AudioCycleOutputTask {
    pub fn start(config: AudioOutputConfig) -> Result<Self, String> {
        let status = Arc::new(Mutex::new(initial_status(config)));
        let diagnostic = Arc::new(Mutex::new(AudioLowFrequencyDiagnosticSnapshot::empty(
            config.sample_rate_hz,
        )));
        let failure = Arc::new(Mutex::new(None));
        let (commands, receiver) = mpsc::sync_channel(8);
        let (startup, startup_receiver) = mpsc::channel();
        let worker_status = Arc::clone(&status);
        let worker_diagnostic = Arc::clone(&diagnostic);
        let worker_failure = Arc::clone(&failure);
        let handle = thread::Builder::new()
            .name("autolive-audio-cycle-output".to_owned())
            .spawn(move || {
                let mut output = autolive_portaudio_output::PortAudioOutput::new(
                    config.sample_rate_hz,
                    config.memory_buffer_kib,
                    config.channels,
                );
                output.set_device_index(config.device_index);
                let startup_result = output
                    .set_frames_per_buffer(config.frames_per_buffer)
                    .and_then(|()| {
                        output.set_callback_paused(true);
                        output.start()
                    });
                if let Err(error) = startup_result {
                    let _ = startup.send(Err(error));
                    return;
                }
                update_status(&worker_status, &output, None);
                let _ = startup.send(Ok(()));
                output_loop(
                    receiver,
                    output,
                    worker_status,
                    worker_diagnostic,
                    worker_failure,
                );
            })
            .map_err(|error| format!("启动音频周期输出线程失败：{error}"))?;
        match startup_receiver.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                control: AudioCycleOutputControl {
                    commands,
                    status,
                    diagnostic,
                    failure,
                },
                handle: Some(handle),
            }),
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(_) => {
                let (response, receiver) = mpsc::channel();
                let _ = commands.try_send(OutputCommand::Stop(response));
                let _ = receiver.recv_timeout(CONTROL_TIMEOUT);
                let _ = handle.join();
                Err("PortAudio 输出线程未在 5000ms 内完成启动".to_owned())
            }
        }
    }

    pub fn control(&self) -> AudioCycleOutputControl {
        self.control.clone()
    }

    pub fn shutdown(mut self) -> Result<(), String> {
        let _ = self.control.stop();
        if let Some(handle) = self.handle.take() {
            if handle.join().is_err() {
                return Err("音频周期输出线程异常退出".to_owned());
            }
        }
        Ok(())
    }
}

impl Drop for AudioCycleOutputTask {
    fn drop(&mut self) {
        if self.handle.is_none() {
            return;
        }
        let _ = self.control.stop();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl AudioCycleOutputControl {
    pub fn set_media_gain(&self, gain: f32) -> Result<(), String> {
        self.request(|response| OutputCommand::SetMediaGain { gain, response })
    }

    pub fn set_current(
        &self,
        track: AudioMixerTrack,
        timeline: AudioTrackTimeline,
    ) -> Result<(), String> {
        self.request(|response| OutputCommand::SetCurrent {
            track,
            timeline,
            response,
        })
    }

    pub fn crossfade_to(
        &self,
        track: AudioMixerTrack,
        timeline: AudioTrackTimeline,
    ) -> Result<(), String> {
        self.request(|response| OutputCommand::CrossfadeTo {
            track,
            timeline,
            response,
        })
    }

    pub fn play_test_tone(&self, tone: AudioTestTone) -> Result<(), String> {
        self.request(|response| OutputCommand::PlayTestTone { tone, response })
    }

    pub fn start_interlude(
        &self,
        track: AudioMixerTrack,
        config: AudioInterludeMixConfig,
    ) -> Result<(), String> {
        self.request(|response| OutputCommand::StartInterlude {
            track,
            config,
            response,
        })
    }

    pub fn crossfade_interlude_to(&self, track: AudioMixerTrack) -> Result<(), String> {
        self.request(|response| OutputCommand::CrossfadeInterludeTo { track, response })
    }

    pub fn set_interlude_gain(&self, gain: f32) -> Result<(), String> {
        self.request(|response| OutputCommand::SetInterludeGain { gain, response })
    }

    pub fn pause_interlude(&self) -> Result<(), String> {
        self.request(OutputCommand::PauseInterlude)
    }

    pub fn resume_interlude(&self) -> Result<(), String> {
        self.request(OutputCommand::ResumeInterlude)
    }

    pub fn stop_interlude(&self) -> Result<(), String> {
        self.request(OutputCommand::StopInterlude)
    }

    pub fn pause(&self) -> Result<(), String> {
        self.request(OutputCommand::Pause)
    }

    pub fn resume(&self) -> Result<(), String> {
        self.request(OutputCommand::Resume)
    }

    pub fn clear(&self) -> Result<(), String> {
        self.request(OutputCommand::Clear)
    }

    fn stop(&self) -> Result<(), String> {
        let (response, receiver) = mpsc::channel();
        self.commands
            .send(OutputCommand::Stop(response))
            .map_err(|error| format!("音频输出控制队列不可用：{error}"))?;
        receiver
            .recv_timeout(CONTROL_TIMEOUT)
            .map_err(|_| "音频输出线程未在 1000ms 内确认停止".to_owned())?
    }

    pub fn status(&self) -> Option<AudioCycleOutputStatus> {
        self.status.lock().ok().map(|status| *status)
    }

    pub fn failure(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|failure| failure.clone())
    }

    pub fn diagnostic(&self) -> Option<AudioLowFrequencyDiagnosticSnapshot> {
        self.diagnostic
            .lock()
            .ok()
            .map(|diagnostic| diagnostic.clone())
    }

    fn request(
        &self,
        command: impl FnOnce(ControlResponse) -> OutputCommand,
    ) -> Result<(), String> {
        let (response, receiver) = mpsc::channel();
        self.commands
            .try_send(command(response))
            .map_err(|error| format!("音频输出控制队列不可用：{error}"))?;
        receiver
            .recv_timeout(CONTROL_TIMEOUT)
            .map_err(|_| "音频输出线程未在 1000ms 内确认操作".to_owned())?
    }
}

fn output_loop(
    commands: mpsc::Receiver<OutputCommand>,
    mut output: autolive_portaudio_output::PortAudioOutput,
    status: Arc<Mutex<AudioCycleOutputStatus>>,
    diagnostic: Arc<Mutex<AudioLowFrequencyDiagnosticSnapshot>>,
    failure: Arc<Mutex<Option<String>>>,
) {
    let sample_rate_hz = output.sample_rate_hz();
    let output_chunk_samples = OUTPUT_CHUNK_FRAMES * OUTPUT_CHANNELS;
    let output_channels = usize::from(output.channels().max(1));
    let initial_prime_output_samples = output.target_playback_watermark_samples();
    let Some(initial_prime_samples) =
        stereo_samples_for_output_samples(initial_prime_output_samples, output_channels)
    else {
        set_failure(&failure, "PortAudio 初始水位未按完整硬件帧对齐".to_owned());
        output.stop();
        update_status(&status, &output, None);
        return;
    };
    let initial_prime_ms =
        stereo_samples_duration_ms(initial_prime_samples, sample_rate_hz, OUTPUT_CHANNELS);
    let crossfade_samples = stereo_samples_for_ms(sample_rate_hz, AUDIO_CYCLE_CROSSFADE_MS);
    let minimum_candidate_samples =
        stereo_samples_for_ms(sample_rate_hz, AUDIO_CANDIDATE_COMMIT_TAIL_MS);
    let mut current: Option<AudioMixerTrack> = None;
    let mut pending_crossfade: Option<PendingCrossfade> = None;
    let mut pending_output = VecDeque::<f32>::new();
    let mut tone_state: Option<ToneState> = None;
    let mut interlude_state: Option<InterludeMixState> = None;
    let mut timeline = OutputTimeline::default();
    let mut diagnostic_analyzer = LowFrequencyDiagnosticAnalyzer::new(sample_rate_hz);
    let mut paused = output.is_callback_paused();
    let mut media_gain = 1.0_f32;
    let mut last_status_refresh = Instant::now();
    let output_retry_interval = output_block_interval(sample_rate_hz, output.frames_per_buffer());

    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                OutputCommand::SetMediaGain { gain, response } => {
                    media_gain = sanitize_gain(gain, 1.0);
                    let _ = response.send(Ok(()));
                }
                OutputCommand::SetCurrent {
                    track,
                    timeline: next_timeline,
                    response,
                } => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出被新当前轨替换");
                    pending_output.clear();
                    tone_state = None;
                    diagnostic_analyzer.reset();
                    publish_diagnostic(&diagnostic, &mut diagnostic_analyzer);
                    if track.available_samples() < initial_prime_samples {
                        let _ = response.send(Err(format!(
                            "当前音轨不足 {initial_prime_ms}ms，不能启动单一输出混音器"
                        )));
                        continue;
                    }
                    output.set_callback_paused(true);
                    paused = true;
                    if let Err(error) = clear_ring_synchronously(&mut output) {
                        let _ = response.send(Err(error));
                        continue;
                    }
                    current = None;
                    match track.take_exact(initial_prime_samples) {
                        Ok(Some(mut samples)) => {
                            apply_media_gain(&mut samples, media_gain);
                            if let Some(interlude) = interlude_state.as_mut() {
                                match apply_interlude_mix(&mut samples, interlude) {
                                    Ok(true) => {}
                                    Ok(false) => interlude_state = None,
                                    Err(error) => {
                                        let _ = response.send(Err(error));
                                        continue;
                                    }
                                }
                            }
                            match output.prime_stereo_interleaved_available(&samples) {
                                Ok(written) if written == samples.len() => {
                                    diagnostic_analyzer.observe_stereo_pcm(&samples);
                                    current = Some(track);
                                    let health = output.stream_health();
                                    timeline.active = Some(TimelineAnchor {
                                        callback_frame: health.callback_pcm_frames_total,
                                        media_position_ms: next_timeline.media_position_ms,
                                        playback_rate: normalized_playback_rate(
                                            next_timeline.playback_rate,
                                        ),
                                    });
                                    timeline.pending = None;
                                    let _ = response.send(Ok(()));
                                }
                                Ok(_) => {
                                    let _ = response.send(Err(format!(
                                        "PortAudio 环缓无法原子接收初始 {initial_prime_ms}ms PCM"
                                    )));
                                }
                                Err(error) => {
                                    let _ = response.send(Err(error));
                                }
                            }
                        }
                        Ok(None) => {
                            let _ = response.send(Err(format!(
                                "当前音轨在提交期间意外不足 {initial_prime_ms}ms"
                            )));
                        }
                        Err(error) => {
                            let _ = response.send(Err(error));
                        }
                    }
                }
                OutputCommand::CrossfadeTo {
                    track,
                    timeline: next_timeline,
                    response,
                } => {
                    if current.is_none() {
                        let _ = response.send(Err("当前音轨不存在，不能执行交叉淡化".to_owned()));
                    } else if paused {
                        let _ = response.send(Err("音频输出已暂停，不能执行交叉淡化".to_owned()));
                    } else if pending_crossfade.is_some() || tone_state.is_some() {
                        let _ = response.send(Err("音频输出正在执行其他切换".to_owned()));
                    } else if track.available_samples() < minimum_candidate_samples {
                        let _ = response.send(Err(format!(
                            "候选音轨不足 {}ms（{}ms 淡化 + {}ms 尾部）",
                            AUDIO_CANDIDATE_COMMIT_TAIL_MS,
                            AUDIO_CYCLE_CROSSFADE_MS,
                            AUDIO_POST_CROSSFADE_TAIL_MS,
                        )));
                    } else {
                        pending_crossfade = Some(PendingCrossfade {
                            next: track,
                            timeline: next_timeline,
                            response,
                            deadline: Instant::now() + CROSSFADE_WAIT_TIMEOUT,
                        });
                    }
                }
                OutputCommand::PlayTestTone { tone, response } => {
                    if pending_crossfade.is_some()
                        || tone_state.is_some()
                        || interlude_state.is_some()
                    {
                        let _ = response.send(Err("音频输出正在切换、插话或播放测试音".to_owned()));
                        continue;
                    }
                    pending_output.clear();
                    let restore_paused = paused;
                    output.set_callback_paused(true);
                    if let Err(error) = clear_ring_synchronously(&mut output) {
                        let _ = response.send(Err(error));
                        continue;
                    }
                    let tone_samples = build_test_tone(sample_rate_hz, tone);
                    let skipped_track_samples = tone_samples.len();
                    pending_output.extend(tone_samples);
                    tone_state = Some(ToneState {
                        skipped_track_samples,
                        restore_paused,
                    });
                    output.set_callback_paused(false);
                    paused = false;
                    let _ = response.send(Ok(()));
                }
                OutputCommand::StartInterlude {
                    track,
                    config,
                    response,
                } => {
                    if current.is_none() {
                        let _ = response.send(Err(
                            "当前主音轨不存在，不能启动 PortAudio 插话混音".to_owned()
                        ));
                    } else if paused {
                        let _ =
                            response.send(Err("PortAudio 输出已暂停，不能启动插话混音".to_owned()));
                    } else if track.available_samples() < output_chunk_samples
                        && !(track.is_finished() && track.available_samples() >= OUTPUT_CHANNELS)
                    {
                        let _ = response.send(Err("插话 PCM 尚未达到安全启动水位".to_owned()));
                    } else {
                        if let Some(current_interlude) = interlude_state.as_mut() {
                            current_interlude
                                .cancel_pending_crossfade("PortAudio 插话被新插话替换");
                        }
                        interlude_state =
                            Some(InterludeMixState::new(track, config, sample_rate_hz));
                        let _ = response.send(Ok(()));
                    }
                }
                OutputCommand::CrossfadeInterludeTo { track, response } => {
                    let Some(interlude) = interlude_state.as_mut() else {
                        let _ = response.send(Err(
                            "当前没有活动的 PortAudio 插话，不能切换声音预设".to_owned(),
                        ));
                        continue;
                    };
                    interlude.start_crossfade(track, sample_rate_hz, response);
                }
                OutputCommand::SetInterludeGain { gain, response } => {
                    if let Some(interlude) = interlude_state.as_mut() {
                        interlude.set_volume_gain(gain);
                    }
                    let _ = response.send(Ok(()));
                }
                OutputCommand::PauseInterlude(response) => {
                    if let Some(interlude) = interlude_state.as_mut() {
                        interlude.pause();
                    }
                    let _ = response.send(Ok(()));
                }
                OutputCommand::ResumeInterlude(response) => {
                    if let Some(interlude) = interlude_state.as_mut() {
                        interlude.resume();
                    }
                    let _ = response.send(Ok(()));
                }
                OutputCommand::StopInterlude(response) => {
                    if let Some(interlude) = interlude_state.as_mut() {
                        interlude.begin_release();
                    }
                    let _ = response.send(Ok(()));
                }
                OutputCommand::Pause(response) => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出已暂停");
                    paused = true;
                    output.set_callback_paused(true);
                    update_status(&status, &output, Some(&mut timeline));
                    let _ = response.send(Ok(()));
                }
                OutputCommand::Resume(response) => {
                    if let Err(error) =
                        validate_resume_request(current.is_some(), tone_state.is_some())
                    {
                        let _ = response.send(Err(error.to_owned()));
                        continue;
                    }
                    if tone_state.is_none() {
                        paused = false;
                        output.set_callback_paused(false);
                    }
                    update_status(&status, &output, Some(&mut timeline));
                    let _ = response.send(Ok(()));
                }
                OutputCommand::Clear(response) => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出已清空");
                    pending_output.clear();
                    current = None;
                    tone_state = None;
                    if let Some(interlude) = interlude_state.as_mut() {
                        interlude.cancel_pending_crossfade("音频输出已清空");
                    }
                    interlude_state = None;
                    timeline = OutputTimeline::default();
                    diagnostic_analyzer.reset();
                    publish_diagnostic(&diagnostic, &mut diagnostic_analyzer);
                    paused = true;
                    output.set_callback_paused(true);
                    let _ = response.send(clear_ring_synchronously(&mut output));
                }
                OutputCommand::Stop(response) => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出已停止");
                    if let Some(interlude) = interlude_state.as_mut() {
                        interlude.cancel_pending_crossfade("音频输出已停止");
                    }
                    output.set_callback_paused(true);
                    output.clear_ring();
                    output.stop();
                    diagnostic_analyzer.reset();
                    publish_diagnostic(&diagnostic, &mut diagnostic_analyzer);
                    update_status(&status, &output, Some(&mut timeline));
                    let _ = response.send(Ok(()));
                    return;
                }
            }
        }

        if last_status_refresh.elapsed() >= STATUS_REFRESH_INTERVAL {
            update_status(&status, &output, Some(&mut timeline));
            publish_diagnostic(&diagnostic, &mut diagnostic_analyzer);
            last_status_refresh = Instant::now();
        }

        // 初次 SetCurrent 需要在 callback 暂停时填入完整有效水位；
        // 仅在没有待写数据时暂停继续消费当前轨。
        if tone_state.is_none() && paused && pending_output.is_empty() {
            thread::sleep(output_retry_interval);
            continue;
        }

        if pending_output.is_empty() {
            if let Some(tone) = tone_state.take() {
                if output.ring_len_samples() > 0 {
                    tone_state = Some(tone);
                    thread::sleep(output_retry_interval);
                    continue;
                }
                if let Some(track) = current.as_ref() {
                    let _ = track.take_up_to(tone.skipped_track_samples);
                }
                paused = tone.restore_paused;
                output.set_callback_paused(paused);
                continue;
            }

            if let Some(switch) = pending_crossfade.take() {
                let Some(old) = current.as_ref() else {
                    let _ = switch
                        .response
                        .send(Err("交叉淡化期间当前音轨已经不存在".to_owned()));
                    continue;
                };
                if let Err(error) =
                    validate_candidate_signal(old, &switch.next, minimum_candidate_samples)
                {
                    let _ = switch.response.send(Err(error));
                    continue;
                }
                let vacant_output_samples = output
                    .ring_capacity_samples()
                    .saturating_sub(output.ring_len_samples());
                let vacant_stereo_samples = stereo_samples_for_output_samples(
                    vacant_output_samples,
                    usize::from(output.channels().max(1)),
                )
                .unwrap_or(0);
                if vacant_stereo_samples < crossfade_samples {
                    if Instant::now() < switch.deadline {
                        pending_crossfade = Some(switch);
                    } else {
                        let _ = switch.response.send(Err(format!(
                            "PortAudio 环缓未在 {}ms 内提供完整 {}ms 淡化空间",
                            CROSSFADE_WAIT_TIMEOUT.as_millis(),
                            AUDIO_CYCLE_CROSSFADE_MS,
                        )));
                    }
                    thread::sleep(output_retry_interval);
                    continue;
                }
                let fade_in_from_silence = output.ring_len_samples() == 0;
                match take_crossfade_pair_or_fade_in(
                    old,
                    &switch.next,
                    crossfade_samples,
                    fade_in_from_silence,
                ) {
                    Ok(Some((old_samples, next_samples))) => {
                        let mut mixed = linear_crossfade(&old_samples, &next_samples);
                        apply_media_gain(&mut mixed, media_gain);
                        if let Some(interlude) = interlude_state.as_mut() {
                            match apply_interlude_mix(&mut mixed, interlude) {
                                Ok(true) => {}
                                Ok(false) => interlude_state = None,
                                Err(error) => {
                                    let _ = switch.response.send(Err(error));
                                    continue;
                                }
                            }
                        }
                        match output.prime_stereo_interleaved_available(&mixed) {
                            Ok(written) if written == mixed.len() => {
                                diagnostic_analyzer.observe_stereo_pcm(&mixed);
                                current = Some(switch.next);
                                let health = output.stream_health();
                                let ring_frames_before_fade = u64::try_from(
                                    output.ring_len_samples().saturating_sub(mixed.len())
                                        / usize::from(output.channels().max(1)),
                                )
                                .unwrap_or(u64::MAX);
                                timeline.pending = Some(TimelineAnchor {
                                    callback_frame: health
                                        .callback_pcm_frames_total
                                        .saturating_add(ring_frames_before_fade),
                                    media_position_ms: switch.timeline.media_position_ms,
                                    playback_rate: normalized_playback_rate(
                                        switch.timeline.playback_rate,
                                    ),
                                });
                                let _ = switch.response.send(Ok(()));
                            }
                            Ok(_) => {
                                let _ = switch
                                    .response
                                    .send(Err("PortAudio 环缓未能原子提交完整交叉淡化".to_owned()));
                            }
                            Err(error) => {
                                let _ = switch.response.send(Err(error));
                            }
                        }
                    }
                    Ok(None) if Instant::now() < switch.deadline => {
                        if let Err(error) = refill_pending_output_from_current(
                            old,
                            &mut pending_output,
                            output_chunk_samples,
                        ) {
                            let _ = switch.response.send(Err(error.clone()));
                            set_failure(&failure, error);
                            output.stop();
                            update_status(&status, &output, Some(&mut timeline));
                            return;
                        }
                        if let Err(error) = mix_pending_output_once(
                            &mut pending_output,
                            media_gain,
                            &mut interlude_state,
                        ) {
                            let _ = switch.response.send(Err(error.clone()));
                            set_failure(&failure, error);
                            output.stop();
                            update_status(&status, &output, Some(&mut timeline));
                            return;
                        }
                        pending_crossfade = Some(switch);
                    }
                    Ok(None) => {
                        let _ = switch.response.send(Err(format!(
                            "当前轨未在 {}ms 内提供 {}ms 交叉淡化 PCM",
                            CROSSFADE_WAIT_TIMEOUT.as_millis(),
                            AUDIO_CYCLE_CROSSFADE_MS,
                        )));
                    }
                    Err(error) => {
                        let _ = switch.response.send(Err(error));
                    }
                }
            } else if let Some(track) = current.as_ref() {
                if let Err(error) = refill_pending_output_from_current(
                    track,
                    &mut pending_output,
                    output_chunk_samples,
                ) {
                    set_failure(&failure, error);
                    output.stop();
                    update_status(&status, &output, Some(&mut timeline));
                    return;
                }
                if let Err(error) =
                    mix_pending_output_once(&mut pending_output, media_gain, &mut interlude_state)
                {
                    set_failure(&failure, error);
                    output.stop();
                    update_status(&status, &output, Some(&mut timeline));
                    return;
                }
            }
        }

        if pending_output.is_empty() {
            thread::sleep(output_retry_interval);
            continue;
        }

        let writable_samples = output
            .writable_stereo_samples_within_watermark()
            .min(output_chunk_samples);
        let write_result = if writable_samples < OUTPUT_CHANNELS {
            None
        } else {
            let write_chunk = pending_stereo_chunk(&mut pending_output, writable_samples);
            Some(
                output
                    .write_stereo_interleaved_available(write_chunk)
                    .map(|written| {
                        let observed = written.min(write_chunk.len());
                        diagnostic_analyzer.observe_stereo_pcm(&write_chunk[..observed]);
                        observed
                    }),
            )
        };
        match write_result {
            None | Some(Ok(0)) => thread::sleep(output_retry_interval),
            Some(Ok(observed)) => {
                pending_output.drain(..observed.min(pending_output.len()));
            }
            Some(Err(error)) => {
                fail_pending_switch(&mut pending_crossfade, &error);
                set_failure(&failure, error);
                output.stop();
                update_status(&status, &output, Some(&mut timeline));
                return;
            }
        }
    }
}

fn mix_pending_output_once(
    pending_output: &mut VecDeque<f32>,
    media_gain: f32,
    interlude_state: &mut Option<InterludeMixState>,
) -> Result<(), String> {
    apply_media_gain(pending_output.make_contiguous(), media_gain);
    let Some(interlude) = interlude_state.as_mut() else {
        return Ok(());
    };
    let still_active = apply_interlude_mix(pending_output.make_contiguous(), interlude)?;
    if !still_active {
        *interlude_state = None;
    }
    Ok(())
}

fn refill_pending_output_from_current(
    current: &AudioMixerTrack,
    pending_output: &mut VecDeque<f32>,
    output_chunk_samples: usize,
) -> Result<(), String> {
    if pending_output.is_empty() {
        pending_output.extend(current.take_up_to(output_chunk_samples)?);
    }
    Ok(())
}

fn pending_stereo_chunk(pending: &mut VecDeque<f32>, max_samples: usize) -> &[f32] {
    let max_samples = max_samples - max_samples % OUTPUT_CHANNELS;
    let front_len = {
        let (front, _) = pending.as_slices();
        front.len().min(max_samples) / OUTPUT_CHANNELS * OUTPUT_CHANNELS
    };
    if front_len >= OUTPUT_CHANNELS {
        let (front, _) = pending.as_slices();
        return &front[..front_len];
    }

    // 环尾可能只剩半个立体声帧；仅此时整理一次，正常热路径直接借用前片。
    let contiguous = pending.make_contiguous();
    let chunk_len = contiguous.len().min(max_samples) / OUTPUT_CHANNELS * OUTPUT_CHANNELS;
    &contiguous[..chunk_len]
}

fn validate_candidate_signal(
    current: &AudioMixerTrack,
    candidate: &AudioMixerTrack,
    probe_samples: usize,
) -> Result<(), String> {
    let current_available = current.available_samples();
    let current_peak = current.peak_in_first(probe_samples)?.unwrap_or(0.0);
    let candidate_peak = candidate.peak_in_first(probe_samples)?.unwrap_or(0.0);
    if candidate_peak <= DIGITAL_SILENCE_PEAK
        && (current_available < probe_samples || current_peak >= AUDIBLE_REFERENCE_PEAK)
    {
        return Err(AUDIO_CANDIDATE_DIGITAL_SILENCE_ERROR.to_owned());
    }
    Ok(())
}

fn take_crossfade_pair(
    old: &AudioMixerTrack,
    next: &AudioMixerTrack,
    sample_count: usize,
) -> Result<Option<CrossfadePair>, String> {
    // 此输出线程是两个缓冲的唯一消费者；producer 只追加尾部。
    if old.available_samples() < sample_count || next.available_samples() < sample_count {
        return Ok(None);
    }
    let Some(old_samples) = old.take_exact(sample_count)? else {
        return Ok(None);
    };
    let Some(next_samples) = next.take_exact(sample_count)? else {
        return Err("候选 PCM 在交叉淡化提交期间意外不足".to_owned());
    };
    Ok(Some((old_samples, next_samples)))
}

fn take_crossfade_pair_or_fade_in(
    old: &AudioMixerTrack,
    next: &AudioMixerTrack,
    sample_count: usize,
    fade_in_from_silence: bool,
) -> Result<Option<CrossfadePair>, String> {
    if old.available_samples() >= sample_count {
        return take_crossfade_pair(old, next, sample_count);
    }
    if !fade_in_from_silence {
        return Ok(None);
    }
    let Some(next_samples) = next.take_exact(sample_count)? else {
        return Ok(None);
    };
    Ok(Some((vec![0.0; sample_count], next_samples)))
}

fn apply_interlude_mix(
    main_samples: &mut [f32],
    state: &mut InterludeMixState,
) -> Result<bool, String> {
    debug_assert_eq!(main_samples.len() % OUTPUT_CHANNELS, 0);
    if state.paused {
        return Ok(true);
    }
    let interlude_samples = state.take_interlude_samples(main_samples.len())?;
    let finite_track_finished = state.pending_crossfade.is_none() && state.track.is_finished();
    let frame_count = main_samples.len() / OUTPUT_CHANNELS;
    let mut active = true;
    for frame in 0..frame_count {
        if finite_track_finished
            && frame.saturating_mul(OUTPUT_CHANNELS) >= interlude_samples.len()
            && !matches!(&state.envelope, InterludeEnvelope::Release { .. })
        {
            state.begin_release();
        }
        let (envelope_gain, still_active) = state.envelope_gain();
        active = still_active;
        let main_gain = 1.0 - (1.0 - state.duck_gain) * envelope_gain;
        let interlude_gain = state.volume_gain * envelope_gain;
        for channel in 0..OUTPUT_CHANNELS {
            let index = frame * OUTPUT_CHANNELS + channel;
            let main = finite_sample(main_samples[index]);
            let insert = interlude_samples
                .get(index)
                .copied()
                .map(finite_sample)
                .unwrap_or(0.0);
            main_samples[index] = (main * main_gain + insert * interlude_gain).clamp(-1.0, 1.0);
        }
        if !active {
            break;
        }
    }
    if active
        && finite_track_finished
        && state.track.is_exhausted()
        && !matches!(&state.envelope, InterludeEnvelope::Release { .. })
    {
        state.begin_release();
        active = state.release_frames > 0;
    }
    Ok(active)
}

fn apply_media_gain(samples: &mut [f32], gain: f32) {
    let gain = sanitize_gain(gain, 1.0);
    for sample in samples {
        *sample = (finite_sample(*sample) * gain).clamp(-1.0, 1.0);
    }
}

fn finite_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample
    } else {
        0.0
    }
}

fn sanitize_gain(gain: f32, maximum: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, maximum)
    } else {
        0.0
    }
}

fn frames_for_ms(sample_rate_hz: u32, duration_ms: u64) -> usize {
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(duration_ms)
            .saturating_div(1_000),
    )
    .unwrap_or(usize::MAX)
}

fn linear_crossfade(old: &[f32], next: &[f32]) -> Vec<f32> {
    linear_crossfade_window(old, next, 0, old.len() / OUTPUT_CHANNELS)
}

fn linear_crossfade_window(
    old: &[f32],
    next: &[f32],
    start_frame: usize,
    total_frames: usize,
) -> Vec<f32> {
    debug_assert_eq!(old.len(), next.len());
    debug_assert_eq!(old.len() % OUTPUT_CHANNELS, 0);
    let frame_count = old.len() / OUTPUT_CHANNELS;
    let mut mixed = Vec::with_capacity(old.len());
    for frame in 0..frame_count {
        let progress = if total_frames <= 1 {
            1.0
        } else {
            start_frame.saturating_add(frame) as f32 / (total_frames - 1) as f32
        };
        let old_gain = 1.0 - progress;
        for channel in 0..OUTPUT_CHANNELS {
            let index = frame * OUTPUT_CHANNELS + channel;
            let old_sample = if old[index].is_finite() {
                old[index]
            } else {
                0.0
            };
            let next_sample = if next[index].is_finite() {
                next[index]
            } else {
                0.0
            };
            mixed.push(old_sample * old_gain + next_sample * progress);
        }
    }
    mixed
}

fn build_test_tone(sample_rate_hz: u32, tone: AudioTestTone) -> Vec<f32> {
    let duration_ms = tone.duration_ms.clamp(1, 2_000);
    let frame_count = usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(u64::from(duration_ms))
            .saturating_div(1_000),
    )
    .unwrap_or(usize::MAX);
    let frequency_hz = tone.frequency_hz.clamp(20.0, 20_000.0);
    let amplitude = tone.amplitude.clamp(0.01, 0.4);
    let phase_step = std::f32::consts::TAU * frequency_hz / sample_rate_hz.max(1) as f32;
    let mut samples = Vec::with_capacity(frame_count.saturating_mul(OUTPUT_CHANNELS));
    for frame in 0..frame_count {
        let sample = (frame as f32 * phase_step).sin() * amplitude;
        samples.extend([sample, sample]);
    }
    samples
}

fn validate_resume_request(has_current_track: bool, tone_active: bool) -> Result<(), &'static str> {
    if !has_current_track && !tone_active {
        return Err("当前音轨不存在，拒绝恢复空的 PortAudio 输出");
    }
    Ok(())
}

fn clear_ring_synchronously(
    output: &mut autolive_portaudio_output::PortAudioOutput,
) -> Result<(), String> {
    output.clear_ring();
    let started = Instant::now();
    let retry_interval = output_block_interval(output.sample_rate_hz(), output.frames_per_buffer());
    while output.ring_len_samples() > 0 {
        if started.elapsed() >= RING_CLEAR_TIMEOUT {
            return Err("PortAudio 环缓未在 500ms 内确认清空".to_owned());
        }
        thread::sleep(retry_interval);
    }
    Ok(())
}

fn status_from_output(
    output: &autolive_portaudio_output::PortAudioOutput,
    timeline_media_position_ms: Option<u64>,
) -> AudioCycleOutputStatus {
    AudioCycleOutputStatus {
        health: output.stream_health(),
        sample_rate_hz: output.sample_rate_hz(),
        channels: output.channels(),
        device_index: output.device_index(),
        memory_buffer_kib: output.ring_capacity_kib(),
        frames_per_buffer: output.frames_per_buffer(),
        playback_watermark_ms: stereo_samples_duration_ms(
            output.target_playback_watermark_samples(),
            output.sample_rate_hz(),
            usize::from(output.channels().max(1)),
        ),
        callback_paused: output.is_callback_paused(),
        timeline_media_position_ms,
    }
}

fn initial_status(config: AudioOutputConfig) -> AudioCycleOutputStatus {
    AudioCycleOutputStatus {
        health: autolive_portaudio_output::PortAudioStreamHealth {
            application_running: false,
            hardware_state: autolive_portaudio_output::PortAudioHardwareState::NotCreated,
            output_latency_us: None,
            actual_sample_rate_hz: None,
            callback_count: 0,
            callback_last_status_flags: 0,
            callback_status_flags_count: 0,
            callback_output_buffer_dac_time_delta_us: 0,
            callback_pcm_frames_total: 0,
            xrun_count: 0,
            callback_underrun_count: 0,
            producer_drop_count: 0,
            ring_len_samples: 0,
            ring_capacity_samples: 0,
        },
        sample_rate_hz: config.sample_rate_hz,
        channels: config.channels.max(1),
        device_index: config.device_index,
        memory_buffer_kib: config.memory_buffer_kib,
        frames_per_buffer: config.frames_per_buffer,
        playback_watermark_ms: stereo_samples_duration_ms(
            autolive_portaudio_output::playback_watermark_samples(
                config.sample_rate_hz,
                config.channels,
                config.frames_per_buffer,
            ),
            config.sample_rate_hz,
            usize::from(config.channels.max(1)),
        ),
        callback_paused: true,
        timeline_media_position_ms: None,
    }
}

fn update_status(
    status: &Arc<Mutex<AudioCycleOutputStatus>>,
    output: &autolive_portaudio_output::PortAudioOutput,
    timeline: Option<&mut OutputTimeline>,
) {
    let timeline_media_position_ms =
        timeline.and_then(|timeline| timeline.media_position_ms(output));
    if let Ok(mut status) = status.lock() {
        *status = status_from_output(output, timeline_media_position_ms);
    }
}

fn publish_diagnostic(
    diagnostic: &Arc<Mutex<AudioLowFrequencyDiagnosticSnapshot>>,
    analyzer: &mut LowFrequencyDiagnosticAnalyzer,
) {
    let snapshot = analyzer.snapshot();
    if let Ok(mut current) = diagnostic.lock() {
        *current = snapshot;
    }
}

fn fail_pending_switch(pending: &mut Option<PendingCrossfade>, reason: &str) {
    if let Some(pending) = pending.take() {
        let _ = pending.response.send(Err(reason.to_owned()));
    }
}

fn set_failure(failure: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut failure) = failure.lock() {
        if failure.is_none() {
            *failure = Some(message);
        }
    }
}

fn stereo_samples_for_ms(sample_rate_hz: u32, duration_ms: usize) -> usize {
    let duration_ms = u64::try_from(duration_ms).unwrap_or(u64::MAX);
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(duration_ms)
            .saturating_div(1_000)
            .saturating_mul(OUTPUT_CHANNELS as u64),
    )
    .unwrap_or(usize::MAX)
}

fn stereo_samples_for_output_samples(
    output_samples: usize,
    output_channels: usize,
) -> Option<usize> {
    if output_channels == 0 || !output_samples.is_multiple_of(output_channels) {
        return None;
    }
    output_samples
        .checked_div(output_channels)?
        .checked_mul(OUTPUT_CHANNELS)
}

fn output_block_interval(sample_rate_hz: u32, frames_per_buffer: u32) -> Duration {
    let sample_rate_hz = u64::from(sample_rate_hz.max(1));
    let micros = u64::from(frames_per_buffer.max(1))
        .saturating_mul(1_000_000)
        .saturating_add(sample_rate_hz.saturating_sub(1))
        .saturating_div(sample_rate_hz)
        .max(1);
    Duration::from_micros(micros)
}

fn stereo_samples_duration_ms(samples: usize, sample_rate_hz: u32, channels: usize) -> u64 {
    let samples = u64::try_from(samples).unwrap_or(u64::MAX);
    let channels = u64::try_from(channels.max(1)).unwrap_or(u64::MAX);
    (samples / channels)
        .saturating_mul(1_000)
        .saturating_div(u64::from(sample_rate_hz.max(1)))
}

#[cfg(test)]
mod tests {
    use super::{
        apply_interlude_mix, apply_media_gain, build_test_tone, estimate_media_position_ms,
        linear_crossfade, mix_pending_output_once, output_block_interval, pending_stereo_chunk,
        refill_pending_output_from_current, stereo_samples_for_ms,
        stereo_samples_for_output_samples, take_crossfade_pair_or_fade_in,
        validate_candidate_signal, validate_resume_request, AudioInterludeMixConfig,
        AudioMixerTrack, AudioTestTone, InterludeMixState, TimelineAnchor,
        AUDIO_CYCLE_CROSSFADE_MS,
    };
    use std::collections::VecDeque;

    #[test]
    fn resume_rejects_an_empty_output_track() {
        assert!(validate_resume_request(false, false).is_err());
        assert!(validate_resume_request(true, false).is_ok());
        assert!(validate_resume_request(false, true).is_ok());
    }

    #[test]
    fn crossfade_is_exactly_thirty_milliseconds_at_supported_rates() {
        assert_eq!(
            stereo_samples_for_ms(44_100, AUDIO_CYCLE_CROSSFADE_MS),
            2_646
        );
        assert_eq!(
            stereo_samples_for_ms(48_000, AUDIO_CYCLE_CROSSFADE_MS),
            2_880
        );
    }

    #[test]
    fn hardware_watermark_is_converted_to_internal_stereo_samples() {
        assert_eq!(stereo_samples_for_output_samples(8_820, 1), Some(17_640));
        assert_eq!(stereo_samples_for_output_samples(17_640, 2), Some(17_640));
        assert_eq!(stereo_samples_for_output_samples(3, 2), None);
        assert_eq!(stereo_samples_for_output_samples(1, 0), None);
    }

    #[test]
    fn pending_output_borrows_complete_stereo_frames_across_ring_wrap() {
        let mut pending = VecDeque::from([1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(pending_stereo_chunk(&mut pending, 5), &[1.0, 2.0, 3.0, 4.0]);

        let mut wrapped = VecDeque::with_capacity(5);
        wrapped.extend([0.0, 1.0, 2.0, 3.0, 4.0]);
        wrapped.drain(..4);
        wrapped.extend([5.0, 6.0, 7.0]);
        assert_eq!(pending_stereo_chunk(&mut wrapped, 4), &[4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn output_backpressure_waits_at_least_one_hardware_block() {
        assert_eq!(output_block_interval(48_000, 256).as_micros(), 5_334);
        assert_eq!(output_block_interval(44_100, 2_048).as_micros(), 46_440);
    }

    #[test]
    fn linear_crossfade_has_exact_old_and_new_endpoints() {
        let old = vec![1.0; 8];
        let next = vec![0.0; 8];
        let mixed = linear_crossfade(&old, &next);
        let expected = [
            1.0,
            1.0,
            2.0 / 3.0,
            2.0 / 3.0,
            1.0 / 3.0,
            1.0 / 3.0,
            0.0,
            0.0,
        ];
        assert!(mixed
            .iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < 1e-6));
    }

    #[test]
    fn linear_crossfade_keeps_channels_aligned_and_sanitizes_non_finite_values() {
        let mixed = linear_crossfade(
            &[1.0, -1.0, f32::NAN, f32::INFINITY],
            &[0.0, 0.0, 0.25, -0.25],
        );
        assert_eq!(mixed[0], 1.0);
        assert_eq!(mixed[1], -1.0);
        assert_eq!(mixed[2], 0.25);
        assert_eq!(mixed[3], -0.25);
        assert!(mixed.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn silent_current_track_can_recover_from_a_preheated_candidate() {
        let old = AudioMixerTrack::from_samples(VecDeque::new());
        let next = AudioMixerTrack::from_samples(VecDeque::from([1.0, 1.0, 0.5, 0.5]));

        assert!(take_crossfade_pair_or_fade_in(&old, &next, 4, false)
            .unwrap()
            .is_none());
        let (old_samples, next_samples) = take_crossfade_pair_or_fade_in(&old, &next, 4, true)
            .unwrap()
            .expect("silent recovery must use the ready candidate");
        assert_eq!(old_samples, vec![0.0; 4]);
        assert_eq!(next_samples, vec![1.0, 1.0, 0.5, 0.5]);
    }

    #[test]
    fn current_track_keeps_feeding_output_while_crossfade_waits() {
        let current = AudioMixerTrack::from_samples(VecDeque::from([0.1, 0.1, 0.2, 0.2]));
        let mut pending_output = VecDeque::new();

        refill_pending_output_from_current(&current, &mut pending_output, 2).unwrap();

        assert_eq!(pending_output, VecDeque::from([0.1, 0.1]));
        assert_eq!(current.available_samples(), 2);
    }

    #[test]
    fn audible_current_rejects_a_digitally_silent_candidate_without_consuming_either_track() {
        let current = AudioMixerTrack::from_samples(VecDeque::from([0.25; 16]));
        let candidate = AudioMixerTrack::from_samples(VecDeque::from([0.0; 16]));

        assert!(validate_candidate_signal(&current, &candidate, 16).is_err());
        assert_eq!(current.available_samples(), 16);
        assert_eq!(candidate.available_samples(), 16);
    }

    #[test]
    fn candidate_signal_guard_allows_real_silence_and_a_delayed_fade_in() {
        let silent_current = AudioMixerTrack::from_samples(VecDeque::from([0.0; 16]));
        let silent_candidate = AudioMixerTrack::from_samples(VecDeque::from([0.0; 16]));
        assert!(validate_candidate_signal(&silent_current, &silent_candidate, 16).is_ok());

        let audible_current = AudioMixerTrack::from_samples(VecDeque::from([0.25; 16]));
        let mut fade_in = vec![0.0; 8];
        fade_in.extend([0.01; 8]);
        let fade_in_candidate = AudioMixerTrack::from_samples(VecDeque::from(fade_in));
        assert!(validate_candidate_signal(&audible_current, &fade_in_candidate, 16).is_ok());
    }

    #[test]
    fn missing_current_probe_does_not_authorize_a_silent_candidate() {
        let depleted_current = AudioMixerTrack::from_samples(VecDeque::new());
        let silent_candidate = AudioMixerTrack::from_samples(VecDeque::from([0.0; 16]));

        assert!(validate_candidate_signal(&depleted_current, &silent_candidate, 16).is_err());
        assert_eq!(depleted_current.available_samples(), 0);
        assert_eq!(silent_candidate.available_samples(), 16);
    }

    #[test]
    fn test_tone_is_bounded_stereo_pcm() {
        let tone = build_test_tone(
            44_100,
            AudioTestTone {
                frequency_hz: 440.0,
                duration_ms: 500,
                amplitude: 0.15,
            },
        );
        assert_eq!(tone.len(), 44_100);
        assert!(tone.iter().all(|sample| sample.is_finite()));
        assert!(tone.iter().all(|sample| sample.abs() <= 0.15));
    }

    #[test]
    fn interlude_mix_ducks_the_main_track_and_adds_the_insert_track() {
        let track = AudioMixerTrack::from_samples(VecDeque::from([0.25, 0.25, 0.5, 0.5]));
        let mut state = InterludeMixState::new(
            track,
            AudioInterludeMixConfig {
                volume_gain: 0.5,
                duck_gain: 0.25,
                attack_ms: 0,
                release_ms: 0,
            },
            48_000,
        );
        let mut main = vec![0.4, 0.4, 0.4, 0.4];

        assert!(apply_interlude_mix(&mut main, &mut state).unwrap());
        assert_eq!(main, vec![0.225, 0.225, 0.35, 0.35]);
    }

    #[test]
    fn media_mute_does_not_mute_the_interlude_track() {
        let track = AudioMixerTrack::from_samples(VecDeque::from([0.5, 0.5]));
        let mut state = Some(InterludeMixState::new(
            track,
            AudioInterludeMixConfig {
                volume_gain: 1.0,
                duck_gain: 0.0,
                attack_ms: 0,
                release_ms: 0,
            },
            48_000,
        ));
        let mut pending = VecDeque::from([0.75, 0.75]);

        mix_pending_output_once(&mut pending, 0.0, &mut state).unwrap();

        assert_eq!(pending, VecDeque::from([0.5, 0.5]));
    }

    #[test]
    fn media_gain_is_finite_and_bounded() {
        let mut samples = [0.5, -0.5, f32::NAN, 2.0];
        apply_media_gain(&mut samples, 0.5);
        assert_eq!(samples, [0.25, -0.25, 0.0, 1.0]);
    }

    #[test]
    fn active_interlude_volume_updates_without_restarting_its_envelope() {
        let track = AudioMixerTrack::from_samples(VecDeque::from([0.5, 0.5]));
        let mut state = InterludeMixState::new(
            track,
            AudioInterludeMixConfig {
                volume_gain: 1.0,
                duck_gain: 1.0,
                attack_ms: 0,
                release_ms: 0,
            },
            48_000,
        );
        state.set_volume_gain(0.25);
        let mut main = vec![0.0, 0.0];

        assert!(apply_interlude_mix(&mut main, &mut state).unwrap());
        assert_eq!(main, vec![0.125, 0.125]);
        assert!(matches!(state.envelope, super::InterludeEnvelope::Sustain));
    }

    #[test]
    fn interlude_candidate_crossfades_and_promotes_only_after_completion() {
        let current = AudioMixerTrack::from_samples(VecDeque::from([1.0; 300]));
        let next = AudioMixerTrack::from_samples(VecDeque::from([0.25; 300]));
        let mut state = InterludeMixState::new(
            current,
            AudioInterludeMixConfig {
                volume_gain: 1.0,
                duck_gain: 1.0,
                attack_ms: 0,
                release_ms: 0,
            },
            1_000,
        );
        let (response, receiver) = std::sync::mpsc::channel();
        state.start_crossfade(next, 1_000, response);
        let mut main = vec![0.0; 60];

        assert!(apply_interlude_mix(&mut main, &mut state).unwrap());
        assert_eq!(receiver.try_recv().unwrap(), Ok(()));
        assert_eq!(main[0], 1.0);
        assert_eq!(main[1], 1.0);
        assert_eq!(main[58], 0.25);
        assert_eq!(main[59], 0.25);
        assert!(state.pending_crossfade.is_none());
    }

    #[test]
    fn rejected_interlude_candidate_keeps_the_current_track_untouched() {
        let current = AudioMixerTrack::from_samples(VecDeque::from([0.5; 300]));
        let next = AudioMixerTrack::from_samples(VecDeque::from([0.0; 300]));
        let mut state = InterludeMixState::new(
            current.clone(),
            AudioInterludeMixConfig {
                volume_gain: 1.0,
                duck_gain: 1.0,
                attack_ms: 0,
                release_ms: 0,
            },
            1_000,
        );
        let (response, receiver) = std::sync::mpsc::channel();

        state.start_crossfade(next, 1_000, response);

        assert!(receiver.try_recv().unwrap().is_err());
        assert_eq!(current.available_samples(), 300);
        assert!(state.pending_crossfade.is_none());
    }

    #[test]
    fn pausing_interlude_cancels_pending_crossfade_and_keeps_current() {
        let current = AudioMixerTrack::from_samples(VecDeque::from([0.5; 300]));
        let next = AudioMixerTrack::from_samples(VecDeque::from([0.25; 300]));
        let mut state = InterludeMixState::new(
            current.clone(),
            AudioInterludeMixConfig {
                volume_gain: 1.0,
                duck_gain: 1.0,
                attack_ms: 0,
                release_ms: 0,
            },
            1_000,
        );
        let (response, receiver) = std::sync::mpsc::channel();
        state.start_crossfade(next, 1_000, response);

        state.pause();

        assert!(receiver.try_recv().unwrap().is_err());
        assert_eq!(current.available_samples(), 300);
        assert!(state.pending_crossfade.is_none());
    }

    #[test]
    fn interlude_pcm_is_consumed_once_when_portaudio_accepts_a_partial_chunk() {
        let track = AudioMixerTrack::from_samples(VecDeque::from([0.2; 8]));
        let mut state = Some(InterludeMixState::new(
            track.clone(),
            AudioInterludeMixConfig {
                volume_gain: 0.5,
                duck_gain: 0.5,
                attack_ms: 0,
                release_ms: 0,
            },
            48_000,
        ));
        let mut pending_output = VecDeque::from([0.4; 8]);

        mix_pending_output_once(&mut pending_output, 1.0, &mut state).unwrap();
        assert_eq!(track.available_samples(), 0);
        assert_eq!(
            pending_output.drain(..2).collect::<Vec<_>>(),
            vec![0.3, 0.3]
        );
        assert_eq!(pending_output, VecDeque::from([0.3; 6]));
    }

    #[test]
    fn finite_interlude_stops_at_eof_instead_of_repeating_short_pcm() {
        let track = AudioMixerTrack::from_finite_samples(VecDeque::from([0.2; 4]));
        let mut state = InterludeMixState::new(
            track.clone(),
            AudioInterludeMixConfig {
                volume_gain: 0.5,
                duck_gain: 0.5,
                attack_ms: 0,
                release_ms: 0,
            },
            48_000,
        );
        let mut main = vec![0.4; 8];

        assert!(!apply_interlude_mix(&mut main, &mut state).unwrap());
        assert_eq!(&main[..4], &[0.3; 4]);
        assert_eq!(&main[4..], &[0.4; 4]);
        assert!(track.is_exhausted());
    }

    #[test]
    fn interlude_release_restores_the_main_track_without_layout_or_clock_side_effects() {
        let track = AudioMixerTrack::from_samples(VecDeque::from([0.4; 8]));
        let mut state = InterludeMixState::new(
            track,
            AudioInterludeMixConfig {
                volume_gain: 0.5,
                duck_gain: 0.5,
                attack_ms: 0,
                release_ms: 1,
            },
            1_000,
        );
        state.begin_release();
        let mut main = vec![0.4, 0.4];

        assert!(!apply_interlude_mix(&mut main, &mut state).unwrap());
        assert_eq!(main, vec![0.4, 0.4]);
    }

    #[test]
    fn interlude_zero_release_restores_the_main_track_immediately() {
        let track = AudioMixerTrack::from_samples(VecDeque::from([0.4; 8]));
        let mut state = InterludeMixState::new(
            track,
            AudioInterludeMixConfig {
                volume_gain: 0.5,
                duck_gain: 0.5,
                attack_ms: 0,
                release_ms: 0,
            },
            48_000,
        );
        state.begin_release();
        let mut main = vec![0.4, 0.4];

        assert!(!apply_interlude_mix(&mut main, &mut state).unwrap());
        assert_eq!(main, vec![0.4, 0.4]);
    }

    #[test]
    fn callback_frames_advance_the_absolute_media_timeline_at_playback_rate() {
        let anchor = TimelineAnchor {
            callback_frame: 1_000,
            media_position_ms: 20_000,
            playback_rate: 2.0,
        };

        assert_eq!(estimate_media_position_ms(anchor, 5_410, 44_100), 20_200);
        assert_eq!(estimate_media_position_ms(anchor, 500, 44_100), 20_000);
    }
}

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

const OUTPUT_CHANNELS: usize = 2;
type CrossfadePair = (Vec<f32>, Vec<f32>);
const OUTPUT_CHUNK_FRAMES: usize = 512;
const OUTPUT_RETRY_INTERVAL: Duration = Duration::from_millis(1);
const CONTROL_TIMEOUT: Duration = Duration::from_millis(1_000);
const STARTUP_TIMEOUT: Duration = Duration::from_millis(5_000);
const CROSSFADE_WAIT_TIMEOUT: Duration = Duration::from_millis(500);
const RING_CLEAR_TIMEOUT: Duration = Duration::from_millis(500);
const STATUS_REFRESH_INTERVAL: Duration = Duration::from_millis(50);
const INITIAL_PRIME_MS: usize = 50;
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

#[derive(Debug)]
enum OutputCommand {
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
        let failure = Arc::new(Mutex::new(None));
        let (commands, receiver) = mpsc::sync_channel(8);
        let (startup, startup_receiver) = mpsc::channel();
        let worker_status = Arc::clone(&status);
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
                output_loop(receiver, output, worker_status, worker_failure);
            })
            .map_err(|error| format!("启动音频周期输出线程失败：{error}"))?;
        match startup_receiver.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                control: AudioCycleOutputControl {
                    commands,
                    status,
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
    failure: Arc<Mutex<Option<String>>>,
) {
    let sample_rate_hz = output.sample_rate_hz();
    let output_chunk_samples = OUTPUT_CHUNK_FRAMES * OUTPUT_CHANNELS;
    let initial_prime_samples = stereo_samples_for_ms(sample_rate_hz, INITIAL_PRIME_MS);
    let crossfade_samples = stereo_samples_for_ms(sample_rate_hz, AUDIO_CYCLE_CROSSFADE_MS);
    let minimum_candidate_samples =
        stereo_samples_for_ms(sample_rate_hz, AUDIO_CANDIDATE_COMMIT_TAIL_MS);
    let mut current: Option<AudioMixerTrack> = None;
    let mut pending_crossfade: Option<PendingCrossfade> = None;
    let mut pending_output = VecDeque::<f32>::new();
    let mut tone_state: Option<ToneState> = None;
    let mut timeline = OutputTimeline::default();
    let mut paused = output.is_callback_paused();
    let mut last_status_refresh = Instant::now();

    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                OutputCommand::SetCurrent {
                    track,
                    timeline: next_timeline,
                    response,
                } => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出被新当前轨替换");
                    pending_output.clear();
                    tone_state = None;
                    if track.available_samples() < initial_prime_samples {
                        let _ = response.send(Err(format!(
                            "当前音轨不足 {INITIAL_PRIME_MS}ms，不能启动单一输出混音器"
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
                        Ok(Some(samples)) => {
                            match output.prime_stereo_interleaved_available(&samples) {
                                Ok(written) if written == samples.len() => {
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
                                    let _ =
                                        response
                                            .send(Err("PortAudio 环缓无法原子接收初始 50ms PCM"
                                                .to_owned()));
                                }
                                Err(error) => {
                                    let _ = response.send(Err(error));
                                }
                            }
                        }
                        Ok(None) => {
                            let _ = response.send(Err(format!(
                                "当前音轨在提交期间意外不足 {INITIAL_PRIME_MS}ms"
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
                    if pending_crossfade.is_some() || tone_state.is_some() {
                        let _ = response.send(Err("音频输出正在切换或播放测试音".to_owned()));
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
                OutputCommand::Pause(response) => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出已暂停");
                    paused = true;
                    output.set_callback_paused(true);
                    let _ = response.send(Ok(()));
                }
                OutputCommand::Resume(response) => {
                    if tone_state.is_none() {
                        paused = false;
                        output.set_callback_paused(false);
                    }
                    let _ = response.send(Ok(()));
                }
                OutputCommand::Clear(response) => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出已清空");
                    pending_output.clear();
                    current = None;
                    tone_state = None;
                    timeline = OutputTimeline::default();
                    paused = true;
                    output.set_callback_paused(true);
                    let _ = response.send(clear_ring_synchronously(&mut output));
                }
                OutputCommand::Stop(response) => {
                    fail_pending_switch(&mut pending_crossfade, "音频输出已停止");
                    output.set_callback_paused(true);
                    output.clear_ring();
                    output.stop();
                    update_status(&status, &output, Some(&mut timeline));
                    let _ = response.send(Ok(()));
                    return;
                }
            }
        }

        if last_status_refresh.elapsed() >= STATUS_REFRESH_INTERVAL {
            update_status(&status, &output, Some(&mut timeline));
            last_status_refresh = Instant::now();
        }

        // 初次 SetCurrent 需要在 callback 暂停时把固定 50ms PCM 填入环缓；
        // 仅在没有待写数据时暂停继续消费当前轨。
        if tone_state.is_none() && paused && pending_output.is_empty() {
            thread::sleep(OUTPUT_RETRY_INTERVAL);
            continue;
        }

        if pending_output.is_empty() {
            if let Some(tone) = tone_state.take() {
                if output.ring_len_samples() > 0 {
                    tone_state = Some(tone);
                    thread::sleep(OUTPUT_RETRY_INTERVAL);
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
                let vacant_samples = output
                    .ring_capacity_samples()
                    .saturating_sub(output.ring_len_samples());
                if vacant_samples < crossfade_samples {
                    if Instant::now() < switch.deadline {
                        pending_crossfade = Some(switch);
                    } else {
                        let _ = switch.response.send(Err(format!(
                            "PortAudio 环缓未在 {}ms 内提供完整 {}ms 淡化空间",
                            CROSSFADE_WAIT_TIMEOUT.as_millis(),
                            AUDIO_CYCLE_CROSSFADE_MS,
                        )));
                    }
                    thread::sleep(OUTPUT_RETRY_INTERVAL);
                    continue;
                }
                match take_crossfade_pair(old, &switch.next, crossfade_samples) {
                    Ok(Some((old_samples, next_samples))) => {
                        let mixed = linear_crossfade(&old_samples, &next_samples);
                        match output.prime_stereo_interleaved_available(&mixed) {
                            Ok(written) if written == mixed.len() => {
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
                match track.take_up_to(output_chunk_samples) {
                    Ok(samples) => pending_output.extend(samples),
                    Err(error) => {
                        set_failure(&failure, error);
                        output.stop();
                        update_status(&status, &output, Some(&mut timeline));
                        return;
                    }
                }
            }
        }

        if pending_output.is_empty() {
            thread::sleep(OUTPUT_RETRY_INTERVAL);
            continue;
        }

        let write_chunk: Vec<f32> = pending_output
            .iter()
            .take(output_chunk_samples)
            .copied()
            .collect();
        match output.write_stereo_interleaved_available(&write_chunk) {
            Ok(0) => thread::sleep(OUTPUT_RETRY_INTERVAL),
            Ok(written) => {
                pending_output.drain(..written.min(pending_output.len()));
            }
            Err(error) => {
                fail_pending_switch(&mut pending_crossfade, &error);
                set_failure(&failure, error);
                output.stop();
                update_status(&status, &output, Some(&mut timeline));
                return;
            }
        }
    }
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

fn linear_crossfade(old: &[f32], next: &[f32]) -> Vec<f32> {
    debug_assert_eq!(old.len(), next.len());
    debug_assert_eq!(old.len() % OUTPUT_CHANNELS, 0);
    let frame_count = old.len() / OUTPUT_CHANNELS;
    let mut mixed = Vec::with_capacity(old.len());
    for frame in 0..frame_count {
        let progress = if frame_count <= 1 {
            1.0
        } else {
            frame as f32 / (frame_count - 1) as f32
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

fn clear_ring_synchronously(
    output: &mut autolive_portaudio_output::PortAudioOutput,
) -> Result<(), String> {
    output.clear_ring();
    let started = Instant::now();
    while output.ring_len_samples() > 0 {
        if started.elapsed() >= RING_CLEAR_TIMEOUT {
            return Err("PortAudio 环缓未在 500ms 内确认清空".to_owned());
        }
        thread::sleep(OUTPUT_RETRY_INTERVAL);
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
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(duration_ms as u64)
            .saturating_div(1_000)
            .saturating_mul(OUTPUT_CHANNELS as u64),
    )
    .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::{
        build_test_tone, estimate_media_position_ms, linear_crossfade, stereo_samples_for_ms,
        AudioTestTone, TimelineAnchor, AUDIO_CYCLE_CROSSFADE_MS,
    };

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

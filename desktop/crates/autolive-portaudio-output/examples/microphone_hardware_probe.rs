//! 显式运行的 Windows 麦克风/扬声器证据探针。
//!
//! 该探针不会在普通 `cargo test` 中运行。只有用户主动执行 `cargo run
//! --example microphone_hardware_probe -- ...` 时才会打开设备；默认只写指标，
//! 传入 `--record` 才会把输入、参考和清理后的 PCM 写入指定目录。

use autolive_portaudio_output::{
    default_input_device, input_device_index_for_id, list_input_devices, PortAudioDuplexConfig,
    PortAudioInputHealth, PortAudioOutput, PortAudioStreamHealth, DEFAULT_FRAMES_PER_BUFFER,
    DEFAULT_RING_CAPACITY_KIB, DEFAULT_SAMPLE_RATE_HZ,
};
use autolive_speech_dsp::{SpeechDsp, SpeechDspConfig};
use std::collections::VecDeque;
use std::env;
use std::f32::consts::PI;
use std::fs::{create_dir_all, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_DURATION_SECONDS: u64 = 10;
const FAR_END_FREQUENCY_HZ: f32 = 440.0;
const DEFAULT_FAR_END_AMPLITUDE: f32 = 0.03;
const MAX_DURATION_SECONDS: u64 = 3_600;

#[derive(Debug)]
struct ProbeOptions {
    duration_seconds: u64,
    far_end_amplitude: f32,
    input_id: Option<String>,
    output_dir: PathBuf,
    record: bool,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            duration_seconds: DEFAULT_DURATION_SECONDS,
            far_end_amplitude: DEFAULT_FAR_END_AMPLITUDE,
            input_id: None,
            output_dir: PathBuf::from("target/microphone-hardware-probe"),
            record: false,
        }
    }
}

#[derive(Debug, Default)]
struct RmsAccumulator {
    sum_squares: f64,
    samples: u64,
}

struct ProbeMetrics<'a> {
    input_id: &'a str,
    input_name: &'a str,
    sample_rate_hz: u32,
    duration: Duration,
    processed_frames: u64,
    speech_frames: u64,
    dropped_output_blocks: u64,
    raw_rms: f64,
    reference_rms: f64,
    clean_rms: f64,
    input: PortAudioInputHealth,
    output: PortAudioStreamHealth,
    recorded: bool,
}

impl RmsAccumulator {
    fn observe(&mut self, sample: f32) {
        if sample.is_finite() {
            self.sum_squares += f64::from(sample) * f64::from(sample);
            self.samples = self.samples.saturating_add(1);
        }
    }

    fn rms(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            (self.sum_squares / self.samples as f64).sqrt()
        }
    }
}

struct WavWriter {
    file: File,
    samples: u32,
}

impl WavWriter {
    fn create(path: PathBuf, sample_rate_hz: u32) -> io::Result<Self> {
        let mut file = File::create(path)?;
        file.write_all(&[0; 44])?;
        let mut writer = Self { file, samples: 0 };
        writer.write_header(sample_rate_hz)?;
        Ok(writer)
    }

    fn write_samples(&mut self, samples: &[f32]) -> io::Result<()> {
        for sample in samples.iter().copied() {
            let value = if sample.is_finite() {
                (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16
            } else {
                0
            };
            self.file.write_all(&value.to_le_bytes())?;
            self.samples = self.samples.saturating_add(1);
        }
        Ok(())
    }

    fn finish(mut self, sample_rate_hz: u32) -> io::Result<()> {
        self.write_header(sample_rate_hz)?;
        self.file.flush()
    }

    fn write_header(&mut self, sample_rate_hz: u32) -> io::Result<()> {
        let data_bytes = self.samples.saturating_mul(2);
        let riff_bytes = 36_u32.saturating_add(data_bytes);
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(b"RIFF")?;
        self.file.write_all(&riff_bytes.to_le_bytes())?;
        self.file.write_all(b"WAVEfmt ")?;
        self.file.write_all(&16_u32.to_le_bytes())?;
        self.file.write_all(&1_u16.to_le_bytes())?;
        self.file.write_all(&1_u16.to_le_bytes())?;
        self.file.write_all(&sample_rate_hz.to_le_bytes())?;
        self.file
            .write_all(&sample_rate_hz.saturating_mul(2).to_le_bytes())?;
        self.file.write_all(&2_u16.to_le_bytes())?;
        self.file.write_all(&16_u16.to_le_bytes())?;
        self.file.write_all(b"data")?;
        self.file.write_all(&data_bytes.to_le_bytes())?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("麦克风硬件探针失败：{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = ProbeOptions::parse(env::args().skip(1))?;
    create_dir_all(&options.output_dir)?;
    let devices = list_input_devices()?;
    if devices.is_empty() {
        return Err("没有可用的 PortAudio 麦克风输入设备".into());
    }
    println!("输入设备：");
    for device in &devices {
        println!(
            "  {} | {} | {:?} | {} ch | {} Hz",
            device.id,
            device.name,
            device.host_api,
            device.max_input_channels,
            device.default_sample_rate_hz
        );
    }

    let (input_device_index, selected_device_id, selected_device_name) =
        match options.input_id.as_ref() {
            Some(id) => {
                let device = devices
                    .iter()
                    .find(|device| device.id == *id)
                    .ok_or_else(|| format!("输入设备 ID 不存在：{id}"))?;
                let index = input_device_index_for_id(id)?
                    .ok_or_else(|| format!("输入设备 ID 已失效：{id}"))?;
                (Some(index), device.id.clone(), device.name.clone())
            }
            None => {
                let (index, device) =
                    default_input_device()?.ok_or("PortAudio 没有系统默认输入设备")?;
                (Some(index), device.id, device.name)
            }
        };
    println!("选择输入：{selected_device_name} ({selected_device_id})");
    println!(
        "持续 {} 秒，参考音 {} Hz / {:.3} amplitude；录音落盘：{}",
        options.duration_seconds, FAR_END_FREQUENCY_HZ, options.far_end_amplitude, options.record
    );

    let mut output = PortAudioOutput::new(DEFAULT_SAMPLE_RATE_HZ, DEFAULT_RING_CAPACITY_KIB, 2);
    output.set_frames_per_buffer(DEFAULT_FRAMES_PER_BUFFER)?;
    output.set_prestart_writes_enabled(true);
    let mut phase_sample = 0_u64;
    let prime_frames = usize::try_from(DEFAULT_SAMPLE_RATE_HZ / 100 * 35)?;
    prime_output(
        &mut output,
        prime_frames,
        &mut phase_sample,
        options.far_end_amplitude,
    )?;
    let mut duplex = output.start_duplex(PortAudioDuplexConfig {
        input_device_index,
        input_channels: 1,
    })?;
    let actual_sample_rate_hz = output
        .stream_health()
        .actual_sample_rate_hz
        .unwrap_or(DEFAULT_SAMPLE_RATE_HZ);
    let frame_samples = usize::try_from(actual_sample_rate_hz / 100)?;
    let mut dsp = SpeechDsp::new(SpeechDspConfig {
        sample_rate_hz: actual_sample_rate_hz,
        frame_samples,
        filter_length_samples: frame_samples.saturating_mul(20),
        ..SpeechDspConfig::default()
    })?;
    let input_channels = usize::from(duplex.input_channels().max(1));
    let output_channels = usize::from(duplex.output_channels().max(1));
    let mut input_pending = VecDeque::new();
    let mut reference_pending = VecDeque::new();
    let mut raw_rms = RmsAccumulator::default();
    let mut clean_rms = RmsAccumulator::default();
    let mut reference_rms = RmsAccumulator::default();
    let mut speech_frames = 0_u64;
    let mut processed_frames = 0_u64;
    let mut dropped_output_blocks = 0_u64;
    let mut input_wav = optional_wav(&options, "input-raw.wav", actual_sample_rate_hz)?;
    let mut reference_wav = optional_wav(&options, "reference.wav", actual_sample_rate_hz)?;
    let mut clean_wav = optional_wav(&options, "clean-mic.wav", actual_sample_rate_hz)?;
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(options.duration_seconds);
    let mut input_block = vec![
        0.0_f32;
        frame_samples
            .saturating_mul(input_channels)
            .saturating_mul(4)
    ];
    let mut reference_block = vec![
        0.0_f32;
        frame_samples
            .saturating_mul(output_channels)
            .saturating_mul(4)
    ];
    let mut input_frame = vec![0.0_f32; frame_samples];
    let mut reference_frame = vec![0.0_f32; frame_samples];
    let mut clean_frame = vec![0.0_f32; frame_samples];

    while Instant::now() < deadline {
        // 维持既有 output watermark，避免探针自身的调度抖动制造 xrun，
        // 从而把硬件问题和测试器背压混在同一份证据里。
        loop {
            let writable_stereo_samples = output.writable_stereo_samples_within_watermark();
            let frames = (writable_stereo_samples / 2).min(256);
            if frames == 0 {
                break;
            }
            let tone = build_stereo_tone(
                frames,
                2,
                &mut phase_sample,
                actual_sample_rate_hz,
                options.far_end_amplitude,
            );
            let written = output.write_stereo_interleaved_available(&tone)?;
            if written < tone.len() {
                dropped_output_blocks = dropped_output_blocks.saturating_add(1);
                if written == 0 {
                    break;
                }
            }
        }
        let input_count = duplex.read_input_interleaved(&mut input_block);
        append_downmixed(
            &mut input_pending,
            &input_block[..input_count],
            input_channels,
        );
        let reference_count = duplex.read_render_reference_interleaved(&mut reference_block);
        append_downmixed(
            &mut reference_pending,
            &reference_block[..reference_count],
            output_channels,
        );
        while input_pending.len() >= frame_samples && reference_pending.len() >= frame_samples {
            pop_frame(&mut input_pending, &mut input_frame);
            pop_frame(&mut reference_pending, &mut reference_frame);
            let result =
                dsp.process_interleaved_mono_f32(&input_frame, &reference_frame, &mut clean_frame)?;
            for ((raw, reference), clean) in input_frame
                .iter()
                .zip(reference_frame.iter())
                .zip(clean_frame.iter())
            {
                raw_rms.observe(*raw);
                reference_rms.observe(*reference);
                clean_rms.observe(*clean);
            }
            if let Some(writer) = input_wav.as_mut() {
                writer.write_samples(&input_frame)?;
            }
            if let Some(writer) = reference_wav.as_mut() {
                writer.write_samples(&reference_frame)?;
            }
            if let Some(writer) = clean_wav.as_mut() {
                writer.write_samples(&clean_frame)?;
            }
            processed_frames = processed_frames.saturating_add(1);
            speech_frames = speech_frames.saturating_add(u64::from(result.speech));
        }
        thread::sleep(Duration::from_millis(4));
    }

    let input_health = duplex.input_health();
    let output_health = output.stream_health();
    drop(duplex);
    output.stop();
    if let Some(writer) = input_wav {
        writer.finish(actual_sample_rate_hz)?;
    }
    if let Some(writer) = reference_wav {
        writer.finish(actual_sample_rate_hz)?;
    }
    if let Some(writer) = clean_wav {
        writer.finish(actual_sample_rate_hz)?;
    }

    let metrics = format_metrics(ProbeMetrics {
        input_id: &selected_device_id,
        input_name: &selected_device_name,
        sample_rate_hz: actual_sample_rate_hz,
        duration: started_at.elapsed(),
        processed_frames,
        speech_frames,
        dropped_output_blocks,
        raw_rms: raw_rms.rms(),
        reference_rms: reference_rms.rms(),
        clean_rms: clean_rms.rms(),
        input: input_health,
        output: output_health,
        recorded: options.record,
    });
    std::fs::write(options.output_dir.join("metrics.json"), metrics)?;
    println!(
        "探针完成，指标：{}",
        options.output_dir.join("metrics.json").display()
    );
    Ok(())
}

impl ProbeOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--record" => options.record = true,
                "--duration-seconds" => {
                    let value = args.next().ok_or("--duration-seconds 缺少值")?;
                    options.duration_seconds =
                        value.parse().map_err(|_| "--duration-seconds 必须是整数")?;
                    if !(1..=MAX_DURATION_SECONDS).contains(&options.duration_seconds) {
                        return Err(format!(
                            "--duration-seconds 必须在 1..={MAX_DURATION_SECONDS} 内"
                        ));
                    }
                }
                "--far-amplitude" => {
                    let value = args.next().ok_or("--far-amplitude 缺少值")?;
                    options.far_end_amplitude =
                        value.parse().map_err(|_| "--far-amplitude 必须是数字")?;
                    if !options.far_end_amplitude.is_finite()
                        || !(0.0..=0.5).contains(&options.far_end_amplitude)
                    {
                        return Err("--far-amplitude 必须在 0..=0.5 内".to_owned());
                    }
                }
                "--input-id" => {
                    let value = args.next().ok_or("--input-id 缺少值")?;
                    options.input_id = Some(value);
                }
                "--output-dir" => {
                    options.output_dir = PathBuf::from(args.next().ok_or("--output-dir 缺少值")?);
                }
                "--help" | "-h" => {
                    println!(
                        "用法：cargo run --example microphone_hardware_probe -- [--duration-seconds N] [--input-id ID] [--far-amplitude 0.03] [--record] [--output-dir DIR]"
                    );
                    std::process::exit(0);
                }
                _ => return Err(format!("未知参数：{arg}")),
            }
        }
        Ok(options)
    }
}

fn prime_output(
    output: &mut PortAudioOutput,
    frames: usize,
    phase_sample: &mut u64,
    amplitude: f32,
) -> Result<(), String> {
    let mut remaining = frames;
    while remaining > 0 {
        let block_frames = remaining.min(256);
        let tone = build_stereo_tone(
            block_frames,
            2,
            phase_sample,
            DEFAULT_SAMPLE_RATE_HZ,
            amplitude,
        );
        let written = output.prime_stereo_interleaved_available(&tone)?;
        if written == 0 {
            return Err("无法为 full-duplex 探针预填充输出环缓".to_owned());
        }
        remaining = remaining.saturating_sub(written / 2);
    }
    Ok(())
}

fn build_stereo_tone(
    frames: usize,
    channels: usize,
    phase_sample: &mut u64,
    sample_rate_hz: u32,
    amplitude: f32,
) -> Vec<f32> {
    let channels = channels.max(1);
    let mut output = Vec::with_capacity(frames.saturating_mul(channels));
    for _ in 0..frames {
        let phase =
            2.0 * PI * FAR_END_FREQUENCY_HZ * (*phase_sample as f32) / sample_rate_hz as f32;
        let sample = amplitude * phase.sin();
        output.extend(std::iter::repeat_n(sample, channels));
        *phase_sample = phase_sample.wrapping_add(1);
    }
    output
}

fn append_downmixed(queue: &mut VecDeque<f32>, samples: &[f32], channels: usize) {
    let channels = channels.max(1);
    for frame in samples.chunks_exact(channels) {
        let sum = frame.iter().copied().sum::<f32>();
        queue.push_back(sum / channels as f32);
    }
}

fn pop_frame(queue: &mut VecDeque<f32>, frame: &mut [f32]) {
    for sample in frame.iter_mut() {
        *sample = queue.pop_front().unwrap_or(0.0);
    }
}

fn optional_wav(
    options: &ProbeOptions,
    name: &str,
    sample_rate_hz: u32,
) -> io::Result<Option<WavWriter>> {
    if options.record {
        WavWriter::create(options.output_dir.join(name), sample_rate_hz).map(Some)
    } else {
        Ok(None)
    }
}

fn format_metrics(metrics: ProbeMetrics<'_>) -> String {
    format!(
        "{{\n  \"input_device_id\": \"{}\",\n  \"input_device_name\": \"{}\",\n  \"sample_rate_hz\": {},\n  \"duration_ms\": {},\n  \"processed_10ms_frames\": {},\n  \"speech_frames\": {},\n  \"raw_input_rms\": {:.9},\n  \"render_reference_rms\": {:.9},\n  \"clean_mic_rms\": {:.9},\n  \"recorded_wav\": {},\n  \"dropped_output_blocks\": {},\n  \"input_callback_count\": {},\n  \"input_frames_captured\": {},\n  \"input_overflow_count\": {},\n  \"render_reference_drop_count\": {},\n  \"callback_last_status_flags\": {},\n  \"output_xrun_count\": {},\n  \"output_callback_count\": {},\n  \"output_latency_us\": {},\n  \"last_input_adc_time_us\": {}\n}}\n",
        json_escape(metrics.input_id),
        json_escape(metrics.input_name),
        metrics.sample_rate_hz,
        metrics.duration.as_millis(),
        metrics.processed_frames,
        metrics.speech_frames,
        metrics.raw_rms,
        metrics.reference_rms,
        metrics.clean_rms,
        metrics.recorded,
        metrics.dropped_output_blocks,
        metrics.input.callback_count,
        metrics.input.input_frames_captured,
        metrics.input.input_overflow_count,
        metrics.input.render_reference_drop_count,
        metrics.input.callback_last_status_flags,
        metrics.output.xrun_count,
        metrics.output.callback_count,
        optional_json_u64(metrics.output.output_latency_us),
        optional_json_i64(metrics.input.last_input_adc_time_us),
    )
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            character if character.is_control() => "?".chars().collect(),
            character => vec![character],
        })
        .collect()
}

fn optional_json_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn optional_json_i64(value: Option<i64>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

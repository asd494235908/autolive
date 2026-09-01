//! WGC → D3D11 Video Processor → staging 回读的 720p30 技术样例。
//!
//! 该样例只测量应用最终效果 HWND 的 GPU 捕获边界，不启动 AkVirtualCamera、
//! 不注册系统设备，也不代表发布门禁已经通过。真实门禁应在 Windows 10/11
//! 目标硬件上运行本样例，并把生成的 JSON 作为不可篡改的证据归档。

use autolive_virtual_camera_native::{CaptureConfig, CaptureGpuFacts, NativeCapturePump};
use serde::Serialize;
use std::env;
use std::fmt;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessesToUpdate, System};

const DEFAULT_SECONDS: u64 = 30;
const MAX_SECONDS: u64 = 7_200;
const POLL_INTERVAL: Duration = Duration::from_millis(2);
const FRAME_BUDGET_US: u64 = 33_333;
const FRAME_INTERVAL_P95_BUDGET_US: u64 = 50_000;
const MIN_FRAME_COVERAGE_PERCENT: u64 = 95;
const TARGET_FPS: u64 = 30;
const PROCESS_RESOURCE_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const MAX_WORKING_SET_PEAK_GROWTH_BYTES: u64 = 64 * 1024 * 1024;
const MAX_VIRTUAL_MEMORY_PEAK_GROWTH_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Arguments {
    hwnd: u64,
    seconds: u64,
    output: Option<PathBuf>,
}

#[derive(Debug)]
enum BenchmarkError {
    Usage(String),
    Io(String),
    Json(String),
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => write!(formatter, "参数错误：{message}"),
            Self::Io(message) => write!(formatter, "写入基准报告失败：{message}"),
            Self::Json(message) => write!(formatter, "生成基准报告失败：{message}"),
        }
    }
}

impl std::error::Error for BenchmarkError {}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Percentiles {
    count: usize,
    avg_us: Option<u64>,
    p50_us: Option<u64>,
    p95_us: Option<u64>,
    p99_us: Option<u64>,
    max_us: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuFactsReport {
    adapter_luid: String,
    adapter_name: String,
    vendor_id: u32,
    device_id: u32,
    feature_level: String,
}

#[derive(Debug, Clone, Copy)]
struct ProcessResourceSample {
    working_set_bytes: u64,
    virtual_memory_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessResourceUsage {
    sample_count: usize,
    initial_working_set_bytes: Option<u64>,
    final_working_set_bytes: Option<u64>,
    peak_working_set_bytes: Option<u64>,
    working_set_delta_bytes: Option<i64>,
    working_set_peak_growth_bytes: Option<u64>,
    initial_virtual_memory_bytes: Option<u64>,
    final_virtual_memory_bytes: Option<u64>,
    peak_virtual_memory_bytes: Option<u64>,
    virtual_memory_delta_bytes: Option<i64>,
    virtual_memory_peak_growth_bytes: Option<u64>,
    growth_within_budget: bool,
}

struct ProcessResourceSampler {
    system: System,
    pid: Pid,
    next_sample_at: Instant,
    samples: Vec<ProcessResourceSample>,
}

impl ProcessResourceSampler {
    fn new() -> Self {
        Self {
            system: System::new(),
            pid: Pid::from_u32(std::process::id()),
            next_sample_at: Instant::now(),
            samples: Vec::new(),
        }
    }

    fn sample_if_due(&mut self, now: Instant) {
        if now < self.next_sample_at {
            return;
        }
        while self.next_sample_at <= now {
            self.next_sample_at += PROCESS_RESOURCE_SAMPLE_INTERVAL;
        }
        let _ = self
            .system
            .refresh_processes(ProcessesToUpdate::Some(&[self.pid]), true);
        let Some(process) = self.system.process(self.pid) else {
            return;
        };
        self.samples.push(ProcessResourceSample {
            working_set_bytes: process.memory(),
            virtual_memory_bytes: process.virtual_memory(),
        });
    }

    fn finish(self) -> ProcessResourceUsage {
        summarize_process_resource_usage(&self.samples)
    }
}

fn summarize_process_resource_usage(samples: &[ProcessResourceSample]) -> ProcessResourceUsage {
    let initial = samples.first().copied();
    let final_sample = samples.last().copied();
    let peak_working_set_bytes = samples.iter().map(|sample| sample.working_set_bytes).max();
    let peak_virtual_memory_bytes = samples
        .iter()
        .map(|sample| sample.virtual_memory_bytes)
        .max();
    let working_set_peak_growth_bytes = initial
        .zip(peak_working_set_bytes)
        .map(|(first, peak)| peak.saturating_sub(first.working_set_bytes));
    let virtual_memory_peak_growth_bytes = initial
        .zip(peak_virtual_memory_bytes)
        .map(|(first, peak)| peak.saturating_sub(first.virtual_memory_bytes));
    let growth_within_budget = initial.is_some()
        && samples.len() >= 2
        && working_set_peak_growth_bytes
            .is_some_and(|growth| growth <= MAX_WORKING_SET_PEAK_GROWTH_BYTES)
        && virtual_memory_peak_growth_bytes
            .is_some_and(|growth| growth <= MAX_VIRTUAL_MEMORY_PEAK_GROWTH_BYTES);

    ProcessResourceUsage {
        sample_count: samples.len(),
        initial_working_set_bytes: initial.map(|sample| sample.working_set_bytes),
        final_working_set_bytes: final_sample.map(|sample| sample.working_set_bytes),
        peak_working_set_bytes,
        working_set_delta_bytes: initial
            .zip(final_sample)
            .map(|(first, last)| signed_delta(last.working_set_bytes, first.working_set_bytes)),
        working_set_peak_growth_bytes,
        initial_virtual_memory_bytes: initial.map(|sample| sample.virtual_memory_bytes),
        final_virtual_memory_bytes: final_sample.map(|sample| sample.virtual_memory_bytes),
        peak_virtual_memory_bytes,
        virtual_memory_delta_bytes: initial.zip(final_sample).map(|(first, last)| {
            signed_delta(last.virtual_memory_bytes, first.virtual_memory_bytes)
        }),
        virtual_memory_peak_growth_bytes,
        growth_within_budget,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkReport {
    schema_version: u32,
    gate: &'static str,
    status: &'static str,
    capture_api: &'static str,
    transport: &'static str,
    gpu_facts: Option<GpuFactsReport>,
    process_resource_usage: Option<ProcessResourceUsage>,
    hwnd: u64,
    width: u32,
    height: u32,
    fps: u32,
    requested_seconds: u64,
    minimum_frames: u64,
    elapsed_ms: u64,
    frames_delivered: u64,
    sequence_advances: u64,
    first_timestamp_100ns: Option<i64>,
    last_timestamp_100ns: Option<i64>,
    timestamp_monotonic: bool,
    readback: Percentiles,
    interframe: Percentiles,
    frame_cadence_within_budget: bool,
    frame_budget_us: u64,
    p99_readback_within_budget: bool,
    gpu_scale_and_color_convert: bool,
    zero_copy: bool,
    release_ready: bool,
    errors: Vec<String>,
}

fn signed_delta(end: u64, start: u64) -> i64 {
    match end.cmp(&start) {
        std::cmp::Ordering::Greater => i64::try_from(end - start).unwrap_or(i64::MAX),
        std::cmp::Ordering::Less => -i64::try_from(start - end).unwrap_or(i64::MAX),
        std::cmp::Ordering::Equal => 0,
    }
}

fn gpu_facts_report(facts: &CaptureGpuFacts) -> GpuFactsReport {
    GpuFactsReport {
        adapter_luid: facts.adapter_luid.clone(),
        adapter_name: facts.adapter_name.clone(),
        vendor_id: facts.vendor_id,
        device_id: facts.device_id,
        feature_level: facts.feature_level.clone(),
    }
}

fn parse_args<I>(args: I) -> Result<Arguments, BenchmarkError>
where
    I: IntoIterator<Item = String>,
{
    let mut hwnd = None;
    let mut seconds = DEFAULT_SECONDS;
    let mut output = None;
    let mut iter = args.into_iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--hwnd" => {
                let value = iter
                    .next()
                    .ok_or_else(|| BenchmarkError::Usage("--hwnd 缺少值".to_owned()))?;
                hwnd = Some(parse_u64(&value, "HWND")?);
            }
            "--seconds" => {
                let value = iter
                    .next()
                    .ok_or_else(|| BenchmarkError::Usage("--seconds 缺少值".to_owned()))?;
                let parsed = value
                    .parse::<u64>()
                    .map_err(|_| BenchmarkError::Usage("--seconds 必须是正整数".to_owned()))?;
                if !(1..=MAX_SECONDS).contains(&parsed) {
                    return Err(BenchmarkError::Usage(format!(
                        "--seconds 必须处于 1..={MAX_SECONDS}"
                    )));
                }
                seconds = parsed;
            }
            "--output" => {
                let value = iter
                    .next()
                    .ok_or_else(|| BenchmarkError::Usage("--output 缺少值".to_owned()))?;
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(BenchmarkError::Usage(
                        "--output 必须使用绝对路径，避免报告写入工作区外的未预期位置".to_owned(),
                    ));
                }
                output = Some(path);
            }
            "--help" | "-h" => {
                return Err(BenchmarkError::Usage(usage().to_owned()));
            }
            unknown => {
                return Err(BenchmarkError::Usage(format!(
                    "不支持的参数 {unknown}；用法：{}",
                    usage()
                )));
            }
        }
    }
    let hwnd = hwnd.ok_or_else(|| BenchmarkError::Usage("必须提供 --hwnd".to_owned()))?;
    if hwnd == 0 {
        return Err(BenchmarkError::Usage("--hwnd 不能为 0".to_owned()));
    }
    Ok(Arguments {
        hwnd,
        seconds,
        output,
    })
}

fn parse_u64(value: &str, label: &str) -> Result<u64, BenchmarkError> {
    let (radix, digits) = value
        .strip_prefix("0x")
        .map_or((10, value), |hex| (16, hex));
    if digits.is_empty() {
        return Err(BenchmarkError::Usage(format!("{label} 不能为空")));
    }
    u64::from_str_radix(digits, radix)
        .map_err(|_| BenchmarkError::Usage(format!("{label} 必须是十进制或 0x 十六进制整数")))
}

fn usage() -> &'static str {
    "virtual_camera_gpu_benchmark --hwnd <HWND> [--seconds 1..7200] [--output <绝对路径>]"
}

fn percentile(sorted: &[u64], rank_percent: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((sorted.len() * rank_percent).saturating_add(99)) / 100;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}

fn summarize(mut samples: Vec<u64>) -> Percentiles {
    samples.sort_unstable();
    let total = samples
        .iter()
        .fold(0_u128, |sum, value| sum.saturating_add(u128::from(*value)));
    let avg_us = (!samples.is_empty())
        .then(|| (total / samples.len() as u128).min(u128::from(u64::MAX)) as u64);
    Percentiles {
        count: samples.len(),
        avg_us,
        p50_us: percentile(&samples, 50),
        p95_us: percentile(&samples, 95),
        p99_us: percentile(&samples, 99),
        max_us: samples.last().copied(),
    }
}

fn duration_us(value: Duration) -> u64 {
    value.as_micros().min(u128::from(u64::MAX)) as u64
}

fn minimum_frames_for_duration(seconds: u64) -> u64 {
    seconds
        .saturating_mul(TARGET_FPS)
        .saturating_mul(MIN_FRAME_COVERAGE_PERCENT)
        / 100
}

fn frame_cadence_within_budget(
    seconds: u64,
    frames_delivered: u64,
    interframe: &Percentiles,
) -> bool {
    frames_delivered >= minimum_frames_for_duration(seconds)
        && interframe
            .p95_us
            .is_some_and(|p95| p95 <= FRAME_INTERVAL_P95_BUDGET_US)
}

fn failed_report(arguments: &Arguments, error: String) -> BenchmarkReport {
    BenchmarkReport {
        schema_version: 1,
        gate: "akvirtualcamera-gpu-benchmark-720p30",
        status: "failed",
        capture_api: "windows_graphics_capture",
        transport: "akvcam_mmap_cpu",
        gpu_facts: None,
        process_resource_usage: None,
        hwnd: arguments.hwnd,
        width: 1280,
        height: 720,
        fps: 30,
        requested_seconds: arguments.seconds,
        minimum_frames: minimum_frames_for_duration(arguments.seconds),
        elapsed_ms: 0,
        frames_delivered: 0,
        sequence_advances: 0,
        first_timestamp_100ns: None,
        last_timestamp_100ns: None,
        timestamp_monotonic: true,
        readback: summarize(Vec::new()),
        interframe: summarize(Vec::new()),
        frame_cadence_within_budget: false,
        frame_budget_us: FRAME_BUDGET_US,
        p99_readback_within_budget: false,
        gpu_scale_and_color_convert: true,
        zero_copy: false,
        release_ready: false,
        errors: vec![error],
    }
}

fn run(arguments: &Arguments) -> Result<BenchmarkReport, BenchmarkError> {
    let mut pump = match NativeCapturePump::start(CaptureConfig {
        final_effect_window_id: arguments.hwnd,
        output_width: 1280,
        output_height: 720,
        output_fps: 30,
    })
    .map_err(|error| error.to_string())
    {
        Ok(pump) => pump,
        Err(error) => return Ok(failed_report(arguments, error)),
    };
    let gpu_facts = gpu_facts_report(pump.gpu_facts());
    let mut process_resource_sampler = ProcessResourceSampler::new();
    let started_at = Instant::now();
    let deadline = Duration::from_secs(arguments.seconds);
    let mut readback_samples = Vec::new();
    let mut interframe_samples = Vec::new();
    let mut first_timestamp = None;
    let mut last_timestamp = None;
    let mut timestamp_monotonic = true;
    let mut frames_delivered = 0_u64;
    let mut sequence_advances = 0_u64;
    let mut last_sequence = None;
    let mut errors = Vec::new();

    while started_at.elapsed() < deadline {
        process_resource_sampler.sample_if_due(Instant::now());
        match pump.try_next_frame() {
            Ok(Some(frame)) => {
                frames_delivered = frames_delivered.saturating_add(1);
                readback_samples.push(duration_us(frame.readback_elapsed));
                if let Some(previous) = last_timestamp {
                    if frame.timestamp_100ns < previous {
                        timestamp_monotonic = false;
                    } else {
                        interframe_samples.push(
                            ((frame.timestamp_100ns.saturating_sub(previous).max(0)) as u64)
                                .saturating_div(10),
                        );
                    }
                }
                first_timestamp.get_or_insert(frame.timestamp_100ns);
                last_timestamp = Some(frame.timestamp_100ns);
                if last_sequence.is_none_or(|previous| frame.sequence > previous) {
                    sequence_advances = sequence_advances.saturating_add(1);
                }
                last_sequence = Some(frame.sequence);
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                errors.push(error.to_string());
                break;
            }
        }
    }
    if let Err(error) = pump.stop() {
        errors.push(error.to_string());
    }
    process_resource_sampler.sample_if_due(Instant::now());
    let process_resource_usage = process_resource_sampler.finish();

    let readback = summarize(readback_samples);
    let interframe = summarize(interframe_samples);
    let frame_cadence_within_budget =
        frame_cadence_within_budget(arguments.seconds, frames_delivered, &interframe);
    let p99_readback_within_budget = readback.p99_us.is_some_and(|p99| p99 <= FRAME_BUDGET_US);
    if frames_delivered == 0 {
        errors.push("基准窗口期间没有收到已完成 GPU 回读的帧".to_owned());
    }
    if !timestamp_monotonic {
        errors.push("捕获时间戳出现倒退".to_owned());
    }
    if !frame_cadence_within_budget {
        errors.push(format!(
            "720p30 帧推进不足或间隔过大：至少需要 {} 帧，且 interframe P95 不得超过 {}µs",
            minimum_frames_for_duration(arguments.seconds),
            FRAME_INTERVAL_P95_BUDGET_US
        ));
    }
    let status = if errors.is_empty() && p99_readback_within_budget && frame_cadence_within_budget {
        "passed"
    } else {
        "failed"
    };
    Ok(BenchmarkReport {
        schema_version: 1,
        gate: "akvirtualcamera-gpu-benchmark-720p30",
        status,
        capture_api: "windows_graphics_capture",
        transport: "akvcam_mmap_cpu",
        gpu_facts: Some(gpu_facts),
        process_resource_usage: Some(process_resource_usage),
        hwnd: arguments.hwnd,
        width: 1280,
        height: 720,
        fps: 30,
        requested_seconds: arguments.seconds,
        minimum_frames: minimum_frames_for_duration(arguments.seconds),
        elapsed_ms: started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        frames_delivered,
        sequence_advances,
        first_timestamp_100ns: first_timestamp,
        last_timestamp_100ns: last_timestamp,
        timestamp_monotonic,
        readback,
        interframe,
        frame_cadence_within_budget,
        frame_budget_us: FRAME_BUDGET_US,
        p99_readback_within_budget,
        gpu_scale_and_color_convert: true,
        zero_copy: false,
        release_ready: false,
        errors,
    })
}

fn emit(report: &BenchmarkReport, output: Option<&PathBuf>) -> Result<(), BenchmarkError> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| BenchmarkError::Json(error.to_string()))?;
    if let Some(path) = output {
        std::fs::write(path, format!("{json}\n"))
            .map_err(|error| BenchmarkError::Io(format!("{}：{error}", path.display())))?;
    }
    println!("{json}");
    Ok(())
}

fn main() {
    let arguments = match parse_args(env::args().skip(1)) {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let report = match run(&arguments) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let failed = report.status != "passed";
    if let Err(error) = emit(&report, arguments.output.as_ref()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_window_and_bounds_duration() {
        let arguments = parse_args([
            "--hwnd".to_owned(),
            "0x10".to_owned(),
            "--seconds".to_owned(),
            "60".to_owned(),
        ])
        .expect("valid arguments");
        assert_eq!(arguments.hwnd, 16);
        assert_eq!(arguments.seconds, 60);
    }

    #[test]
    fn rejects_relative_report_path_and_out_of_range_duration() {
        assert!(parse_args([
            "--hwnd".to_owned(),
            "1".to_owned(),
            "--output".to_owned(),
            "report.json".to_owned(),
        ])
        .is_err());
        assert!(parse_args([
            "--hwnd".to_owned(),
            "1".to_owned(),
            "--seconds".to_owned(),
            "0".to_owned(),
        ])
        .is_err());
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let values = [10, 20, 30, 40];
        assert_eq!(percentile(&values, 50), Some(20));
        assert_eq!(percentile(&values, 95), Some(40));
        assert_eq!(percentile(&[], 99), None);
    }

    #[test]
    fn frame_cadence_requires_95_percent_coverage_and_bounded_interframe_p95() {
        assert_eq!(minimum_frames_for_duration(1), 28);
        let within = Percentiles {
            count: 30,
            avg_us: Some(33_333),
            p50_us: Some(33_333),
            p95_us: Some(45_000),
            p99_us: Some(48_000),
            max_us: Some(60_000),
        };
        assert!(frame_cadence_within_budget(1, 28, &within));
        assert!(!frame_cadence_within_budget(1, 27, &within));
        let over_budget = Percentiles {
            p95_us: Some(FRAME_INTERVAL_P95_BUDGET_US + 1),
            ..within
        };
        assert!(!frame_cadence_within_budget(1, 30, &over_budget));
    }

    #[test]
    fn signed_delta_preserves_direction_without_panicking_on_overflow() {
        assert_eq!(signed_delta(12, 10), 2);
        assert_eq!(signed_delta(10, 12), -2);
        assert_eq!(signed_delta(u64::MAX, 0), i64::MAX);
        assert_eq!(signed_delta(0, u64::MAX), i64::MIN + 1);
    }

    #[test]
    fn process_resource_growth_requires_multiple_samples_and_stays_bounded() {
        let within = summarize_process_resource_usage(&[
            ProcessResourceSample {
                working_set_bytes: 100,
                virtual_memory_bytes: 1_000,
            },
            ProcessResourceSample {
                working_set_bytes: 100 + MAX_WORKING_SET_PEAK_GROWTH_BYTES,
                virtual_memory_bytes: 1_000 + MAX_VIRTUAL_MEMORY_PEAK_GROWTH_BYTES,
            },
        ]);
        assert!(within.growth_within_budget);
        assert_eq!(
            within.working_set_peak_growth_bytes,
            Some(MAX_WORKING_SET_PEAK_GROWTH_BYTES)
        );
        assert_eq!(
            within.virtual_memory_peak_growth_bytes,
            Some(MAX_VIRTUAL_MEMORY_PEAK_GROWTH_BYTES)
        );

        let over = summarize_process_resource_usage(&[
            ProcessResourceSample {
                working_set_bytes: 100,
                virtual_memory_bytes: 1_000,
            },
            ProcessResourceSample {
                working_set_bytes: 100 + MAX_WORKING_SET_PEAK_GROWTH_BYTES + 1,
                virtual_memory_bytes: 1_000,
            },
        ]);
        assert!(!over.growth_within_budget);
        assert!(
            !summarize_process_resource_usage(&[ProcessResourceSample {
                working_set_bytes: 100,
                virtual_memory_bytes: 1_000,
            },])
            .growth_within_budget
        );
    }
}

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const WINDOWS_RELEASE_FILES: [&str; 11] = [
    "x86_64-pc-windows-msvc/binaries/ffmpeg.exe",
    "x86_64-pc-windows-msvc/binaries/ffprobe.exe",
    "x86_64-pc-windows-msvc/binaries/mpv.exe",
    "x86_64-pc-windows-msvc/binaries/spirv-cross-c-shared.dll",
    "x86_64-pc-windows-msvc/binaries/vulkan-1.dll",
    "x86_64-pc-windows-msvc/binaries/licenses/mpv/mpv-runtime-manifest.json",
    "x86_64-pc-windows-msvc/binaries/licenses/mpv/Copyright.txt",
    "x86_64-pc-windows-msvc/binaries/licenses/mpv/GPL-2.0.txt",
    "x86_64-pc-windows-msvc/binaries/licenses/mpv/LGPL-2.1.txt",
    "x86_64-pc-windows-msvc/binaries/licenses/mpv/SOURCE.md",
    "x86_64-pc-windows-msvc/binaries/licenses/mpv/THIRD-PARTY-NOTICES.md",
];

const WINDOWS_MPV_MANIFEST_PATH: &str =
    "x86_64-pc-windows-msvc/binaries/licenses/mpv/mpv-runtime-manifest.json";
const WINDOWS_MPV_RUNTIME_FILES: [(&str, &str); 8] = [
    ("mpv.exe", "x86_64-pc-windows-msvc/binaries/mpv.exe"),
    (
        "spirv-cross-c-shared.dll",
        "x86_64-pc-windows-msvc/binaries/spirv-cross-c-shared.dll",
    ),
    (
        "vulkan-1.dll",
        "x86_64-pc-windows-msvc/binaries/vulkan-1.dll",
    ),
    (
        "legal/Copyright.txt",
        "x86_64-pc-windows-msvc/binaries/licenses/mpv/Copyright.txt",
    ),
    (
        "legal/GPL-2.0.txt",
        "x86_64-pc-windows-msvc/binaries/licenses/mpv/GPL-2.0.txt",
    ),
    (
        "legal/LGPL-2.1.txt",
        "x86_64-pc-windows-msvc/binaries/licenses/mpv/LGPL-2.1.txt",
    ),
    (
        "legal/SOURCE.md",
        "x86_64-pc-windows-msvc/binaries/licenses/mpv/SOURCE.md",
    ),
    (
        "legal/THIRD-PARTY-NOTICES.md",
        "x86_64-pc-windows-msvc/binaries/licenses/mpv/THIRD-PARTY-NOTICES.md",
    ),
];
const REQUIRED_MPV_COMPONENTS: [&str; 3] = ["mpv", "libplacebo", "ffmpeg"];
const REQUIRED_MPV_SUPPLY_EVIDENCE: [(&str, &str); 6] = [
    ("report", "reproducible-build-report.json"),
    (
        "corresponding_source",
        "build-evidence/corresponding-source.tar.zst",
    ),
    ("cyclonedx_sbom", "build-evidence/sbom.cdx.json"),
    ("spdx_sbom", "build-evidence/sbom.spdx.json"),
    ("license_inventory", "build-evidence/license-inventory.txt"),
    (
        "copyright_inventory",
        "build-evidence/copyright-inventory.txt",
    ),
];

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeResourceConfigAction {
    Keep,
    UseDevelopmentOverride(String),
}

pub fn runtime_resource_config_action(
    profile: Option<&str>,
    manifest_exists: bool,
    external_override: Option<&str>,
) -> Result<RuntimeResourceConfigAction, &'static str> {
    if manifest_exists {
        return Ok(RuntimeResourceConfigAction::Keep);
    }
    if matches!(profile, Some("debug" | "test")) {
        return Ok(RuntimeResourceConfigAction::UseDevelopmentOverride(
            merge_development_override(external_override)?,
        ));
    }
    Err("release/custom 构建缺少 runtime-resources.json")
}

fn merge_development_override(external_override: Option<&str>) -> Result<String, &'static str> {
    let mut override_value = match external_override {
        Some(value) => {
            serde_json::from_str(value).map_err(|_| "TAURI_CONFIG 不是有效 JSON merge patch")?
        }
        None => Value::Object(Map::new()),
    };
    let root = override_value
        .as_object_mut()
        .ok_or("TAURI_CONFIG 不能替换 Tauri 配置根对象")?;
    let bundle = root
        .entry("bundle")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("TAURI_CONFIG 不能替换 bundle 对象")?;
    bundle.insert("resources".into(), Value::Array(Vec::new()));
    serde_json::to_string(&override_value).map_err(|_| "无法生成开发 TAURI_CONFIG")
}

pub fn validate_release_runtime_resource_config(
    base_config: &str,
    external_override: Option<&str>,
) -> Result<(), &'static str> {
    let base_config: Value =
        serde_json::from_str(base_config).map_err(|_| "tauri.conf.json 不是有效 JSON")?;
    if !has_expected_config_resources(&base_config) {
        return Err("release/custom 构建要求 bundle.resources 包含运行资源清单和内置资源目录");
    }
    let Some(external_override) = external_override else {
        return Ok(());
    };
    let external_override: Value = serde_json::from_str(external_override)
        .map_err(|_| "TAURI_CONFIG 不是有效 JSON merge patch")?;
    let Some(external_override) = external_override.as_object() else {
        return Err("TAURI_CONFIG 不能替换 Tauri 配置根对象");
    };
    let Some(bundle_override) = external_override.get("bundle") else {
        return Ok(());
    };
    let Some(bundle_override) = bundle_override.as_object() else {
        return Err("TAURI_CONFIG 不能移除或替换 bundle.resources");
    };
    if let Some(resources) = bundle_override.get("resources") {
        if !has_expected_resource_list(resources) {
            return Err("TAURI_CONFIG 不能移除、增加或替换 bundle.resources");
        }
    }
    Ok(())
}

pub fn validate_release_runtime_resource_tree(
    manifest_source: &str,
    embedded_root: &Path,
    expected_target: &str,
    windows_mpv_manifest_reference: Option<&str>,
) -> Result<(), String> {
    let manifest: Value = serde_json::from_str(manifest_source)
        .map_err(|_| "runtime-resources.json 不是有效 JSON".to_owned())?;
    if manifest.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err("runtime-resources.json schema_version 必须为 1".to_owned());
    }
    let manifest_target = manifest
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| "runtime-resources.json 缺少 target".to_owned())?;
    if manifest_target != expected_target {
        return Err(format!(
            "runtime-resources.json target 与 Cargo TARGET 不一致：{manifest_target} != {expected_target}"
        ));
    }
    if expected_target != "x86_64-pc-windows-msvc" {
        return Ok(());
    }
    let windows_mpv_manifest_reference = windows_mpv_manifest_reference
        .ok_or_else(|| "Windows release 构建缺少源码侧 mpv 发布清单锚点".to_owned())?;
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "runtime-resources.json 缺少 files".to_owned())?;
    let mut declared_files = BTreeMap::new();
    for entry in files {
        let relative_path = entry
            .get("relative_path")
            .and_then(Value::as_str)
            .ok_or_else(|| "Windows 发布资源缺少 relative_path".to_owned())?;
        if relative_path.contains('\\')
            || relative_path
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            || !relative_path.starts_with("x86_64-pc-windows-msvc/")
        {
            return Err(format!("Windows 发布资源路径无效：{relative_path}"));
        }
        if declared_files.insert(relative_path, entry).is_some() {
            return Err(format!("Windows 发布资源重复声明：{relative_path}"));
        }
    }
    for required in WINDOWS_RELEASE_FILES {
        if !declared_files.contains_key(required) {
            return Err(format!("Windows 发布资源必须且只能声明一次：{required}"));
        }
    }
    if let Some(path) = declared_files
        .keys()
        .find(|path| !WINDOWS_RELEASE_FILES.contains(path))
    {
        return Err(format!("Windows 发布资源不在固定白名单：{path}"));
    }

    let mut actual_files = BTreeSet::new();
    collect_release_files(embedded_root, embedded_root, &mut actual_files)?;
    let declared_paths = declared_files.keys().copied().collect::<BTreeSet<_>>();
    if let Some(path) = actual_files
        .iter()
        .find(|path| !declared_paths.contains(path.as_str()))
    {
        return Err(format!("Windows 内嵌发布资源未在清单声明：{path}"));
    }
    if let Some(path) = declared_paths
        .iter()
        .find(|path| !actual_files.contains(**path))
    {
        return Err(format!("Windows 发布清单声明的资源缺失：{path}"));
    }

    for (relative_path, entry) in declared_files {
        let declared_size = entry
            .get("size_bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("Windows 发布资源大小无效：{relative_path}"))?;
        if WINDOWS_RELEASE_FILES.contains(&relative_path) && declared_size == 0 {
            return Err(format!("Windows 必需发布资源不能为空：{relative_path}"));
        }
        let declared_hash = entry
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|hash| {
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
            .ok_or_else(|| format!("Windows 发布资源 SHA-256 无效：{relative_path}"))?;
        if entry.get("component").and_then(Value::as_str) != Some("media") {
            return Err(format!("Windows 发布资源 component 无效：{relative_path}"));
        }
        let expected_executable = !relative_path.contains("/binaries/licenses/");
        if entry.get("executable").and_then(Value::as_bool) != Some(expected_executable) {
            return Err(format!("Windows 发布资源 executable 无效：{relative_path}"));
        }
        let path = relative_path
            .split('/')
            .fold(embedded_root.to_path_buf(), |path, segment| {
                path.join(segment)
            });
        let metadata = path
            .metadata()
            .map_err(|_| format!("Windows 内嵌发布资源缺失：{relative_path}"))?;
        if !metadata.is_file() || metadata.len() != declared_size {
            return Err(format!("Windows 内嵌发布资源大小不匹配：{relative_path}"));
        }
        if sha256_file(&path)? != declared_hash {
            return Err(format!(
                "Windows 内嵌发布资源 SHA-256 不匹配：{relative_path}"
            ));
        }
    }
    validate_windows_mpv_manifest(embedded_root, windows_mpv_manifest_reference)
}

fn validate_windows_mpv_manifest(
    embedded_root: &Path,
    source_manifest_reference: &str,
) -> Result<(), String> {
    let manifest_path = release_path(embedded_root, WINDOWS_MPV_MANIFEST_PATH);
    let source = std::fs::read(&manifest_path)
        .map_err(|error| format!("无法读取 Windows mpv 发布清单：{error}"))?;
    if source.as_slice() != source_manifest_reference.as_bytes() {
        return Err("Windows mpv 包内发布清单与源码侧候选清单字节不一致".to_owned());
    }
    let source = std::str::from_utf8(&source)
        .map_err(|_| "Windows mpv 发布清单不是有效 UTF-8".to_owned())?;
    let manifest: Value =
        serde_json::from_str(source).map_err(|_| "Windows mpv 发布清单不是有效 JSON".to_owned())?;
    if manifest.get("schema_version").and_then(Value::as_u64) != Some(2) {
        return Err("Windows mpv 发布清单 schema_version 必须为 2".to_owned());
    }
    validate_json_object_keys(
        &manifest,
        "$",
        &[
            "schema_version",
            "target",
            "build",
            "components",
            "audit",
            "files",
            "supply_evidence",
        ],
    )?;
    if manifest.get("target").and_then(Value::as_str) != Some("x86_64-pc-windows-msvc") {
        return Err("Windows mpv 发布清单 target 无效".to_owned());
    }

    let build = manifest.get("build").unwrap_or(&Value::Null);
    validate_json_object_keys(
        build,
        "build",
        &["scope", "claim", "lock_sha256", "report_sha256"],
    )?;
    if build.get("scope").and_then(Value::as_str) != Some("phase7a_supply_candidate") {
        return Err("Windows mpv 发布清单 build.scope 无效".to_owned());
    }
    if build.get("claim").and_then(Value::as_str) != Some("one_locked_cold_build_candidate") {
        return Err("Windows mpv 发布清单 build.claim 无效".to_owned());
    }
    if !build
        .get("lock_sha256")
        .and_then(Value::as_str)
        .is_some_and(|value| is_lower_hex(value, 64))
    {
        return Err("Windows mpv 发布清单 build.lock_sha256 无效".to_owned());
    }
    let report_hash = build
        .get("report_sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_hex(value, 64))
        .ok_or_else(|| "Windows mpv 发布清单 build.report_sha256 无效".to_owned())?;
    let components = manifest.get("components").unwrap_or(&Value::Null);
    validate_json_object_keys(components, "components", &REQUIRED_MPV_COMPONENTS)?;
    let components = components
        .as_object()
        .ok_or_else(|| "Windows mpv 发布清单对象无效：components".to_owned())?;
    for (name, component) in components {
        let label = format!("components.{name}");
        validate_json_object_keys(component, &label, &["source_ref", "license_expression"])?;
        if !component
            .get("source_ref")
            .and_then(Value::as_str)
            .is_some_and(|value| is_lower_hex(value, 40))
        {
            return Err(format!("Windows mpv 发布清单组件完整提交无效：{name}"));
        }
        if !component
            .get("license_expression")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
        {
            return Err(format!("Windows mpv 发布清单组件许可证无效：{name}"));
        }
    }

    let audit = manifest.get("audit").unwrap_or(&Value::Null);
    validate_json_object_keys(
        audit,
        "audit",
        &[
            "release_review_status",
            "corresponding_source_complete",
            "third_party_notices_reviewed",
        ],
    )?;
    if audit.get("release_review_status").and_then(Value::as_str) != Some("approved")
        || audit
            .get("corresponding_source_complete")
            .and_then(Value::as_bool)
            != Some(true)
        || audit
            .get("third_party_notices_reviewed")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err("Windows mpv 发布清单 release audit 必须为 approved/true/true".to_owned());
    }

    let expected_files = WINDOWS_MPV_RUNTIME_FILES.map(|(name, _)| name);
    let files = manifest.get("files").unwrap_or(&Value::Null);
    validate_json_object_keys(files, "files", &expected_files)?;
    for (manifest_name, relative_path) in WINDOWS_MPV_RUNTIME_FILES {
        let descriptor = files.get(manifest_name).unwrap_or(&Value::Null);
        let (declared_size, declared_hash) =
            validate_manifest_file_descriptor(descriptor, &format!("files.{manifest_name}"))?;
        let path = release_path(embedded_root, relative_path);
        let metadata = path
            .symlink_metadata()
            .map_err(|_| format!("Windows mpv 运行文件缺失：{manifest_name}"))?;
        if !metadata.file_type().is_file() || metadata.len() != declared_size {
            return Err(format!(
                "Windows mpv 发布清单文件大小不匹配：{manifest_name}"
            ));
        }
        if sha256_file(&path)? != declared_hash {
            return Err(format!(
                "Windows mpv 发布清单文件哈希不匹配：{manifest_name}"
            ));
        }
    }

    let supply_evidence = manifest.get("supply_evidence").unwrap_or(&Value::Null);
    let expected_supply_evidence = REQUIRED_MPV_SUPPLY_EVIDENCE.map(|(role, _)| role);
    validate_json_object_keys(
        supply_evidence,
        "supply_evidence",
        &expected_supply_evidence,
    )?;
    for (role, expected_path) in REQUIRED_MPV_SUPPLY_EVIDENCE {
        let actual_path = validate_supply_evidence_descriptor(
            supply_evidence.get(role).unwrap_or(&Value::Null),
            &format!("supply_evidence.{role}"),
        )?;
        if actual_path != expected_path {
            return Err(format!("Windows mpv 发布清单证据角色路径不匹配：{role}"));
        }
    }
    let evidence_report_hash = supply_evidence
        .get("report")
        .and_then(|descriptor| descriptor.get("sha256"))
        .and_then(Value::as_str)
        .ok_or_else(|| "Windows mpv 发布清单 supply_evidence.report.sha256 无效".to_owned())?;
    if evidence_report_hash != report_hash {
        return Err(
            "Windows mpv 发布清单 supply_evidence.report 与 build.report_sha256 不一致".to_owned(),
        );
    }
    Ok(())
}

fn validate_manifest_file_descriptor<'a>(
    value: &'a Value,
    label: &str,
) -> Result<(u64, &'a str), String> {
    validate_json_object_keys(value, label, &["size_bytes", "sha256"])?;
    let size = value
        .get("size_bytes")
        .and_then(Value::as_u64)
        .filter(|size| *size > 0)
        .ok_or_else(|| format!("Windows mpv 发布清单大小无效：{label}"))?;
    let hash = value
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_hex(value, 64))
        .ok_or_else(|| format!("Windows mpv 发布清单 SHA-256 无效：{label}"))?;
    Ok((size, hash))
}

fn validate_supply_evidence_descriptor<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a str, String> {
    validate_json_object_keys(value, label, &["path", "size_bytes", "sha256"])?;
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| is_safe_relative_evidence_path(path))
        .ok_or_else(|| format!("Windows mpv 发布清单证据路径无效：{label}"))?;
    value
        .get("size_bytes")
        .and_then(Value::as_u64)
        .filter(|size| *size > 0)
        .ok_or_else(|| format!("Windows mpv 发布清单证据大小无效：{label}"))?;
    value
        .get("sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_hex(value, 64))
        .ok_or_else(|| format!("Windows mpv 发布清单证据 SHA-256 无效：{label}"))?;
    Ok(path)
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_safe_relative_evidence_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
        && !path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
}

fn validate_json_object_keys(value: &Value, path: &str, expected: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("Windows mpv 发布清单对象无效：{path}"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("Windows mpv 发布清单对象字段不匹配：{path}"));
    }
    Ok(())
}

fn release_path(root: &Path, relative_path: &str) -> std::path::PathBuf {
    relative_path
        .split('/')
        .fold(root.to_path_buf(), |path, segment| path.join(segment))
}

fn collect_release_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
) -> Result<(), String> {
    let entries = directory
        .read_dir()
        .map_err(|error| format!("无法读取内嵌发布资源目录 {}：{error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("无法读取内嵌发布资源项：{error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("无法读取内嵌发布资源类型：{error}"))?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_release_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative_path = path
                .strip_prefix(root)
                .map_err(|_| "内嵌发布资源不在资源根目录内".to_owned())?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative_path);
        } else {
            return Err(format!(
                "内嵌发布资源只允许普通文件和目录：{}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes = file
            .read(&mut buffer)
            .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        if bytes == 0 {
            break;
        }
        hash.update(&buffer[..bytes]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn has_expected_config_resources(config: &Value) -> bool {
    config
        .get("bundle")
        .and_then(Value::as_object)
        .and_then(|bundle| bundle.get("resources"))
        .is_some_and(has_expected_resource_list)
}

fn has_expected_resource_list(value: &Value) -> bool {
    value.as_array().is_some_and(|resources| {
        resources.len() >= 2
            && resources
                .iter()
                .any(|item| item.as_str() == Some("runtime-resources.json"))
            && resources
                .iter()
                .any(|item| item.as_str() == Some("embedded-runtime-resources"))
    })
}

#[path = "../build_support.rs"]
mod build_support;

use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

fn sha256(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn release_path(root: &Path, relative_path: &str) -> std::path::PathBuf {
    relative_path
        .split('/')
        .fold(root.to_path_buf(), |path, segment| path.join(segment))
}

fn write_release_file(root: &Path, relative_path: &str, content: &[u8]) -> serde_json::Value {
    let path = release_path(root, relative_path);
    fs::create_dir_all(path.parent().expect("release file parent")).expect("create parent");
    fs::write(&path, content).expect("write release file");
    serde_json::json!({
        "relative_path": relative_path,
        "sha256": sha256(content),
        "size_bytes": content.len(),
        "executable": relative_path.ends_with(".exe") || relative_path.ends_with(".dll"),
        "component": "media",
    })
}

fn manifest_descriptor(content: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "size_bytes": content.len(),
        "sha256": sha256(content),
    })
}

fn outer_manifest(files: &[serde_json::Value]) -> String {
    serde_json::to_string(&serde_json::json!({
        "schema_version": 1,
        "target": "x86_64-pc-windows-msvc",
        "files": files,
    }))
    .expect("outer manifest JSON")
}

fn mpv_manifest_source(mpv_manifest: &serde_json::Value) -> String {
    serde_json::to_string_pretty(mpv_manifest).expect("mpv manifest JSON")
}

fn validate_windows_release_tree(
    outer_manifest: &str,
    root: &Path,
    source_manifest: &serde_json::Value,
) -> Result<(), String> {
    let source_manifest = mpv_manifest_source(source_manifest);
    build_support::validate_release_runtime_resource_tree(
        outer_manifest,
        root,
        "x86_64-pc-windows-msvc",
        Some(&source_manifest),
    )
}

fn write_windows_release_tree(
    root: &Path,
    required: &[&str],
    mpv_manifest: &serde_json::Value,
) -> (Vec<serde_json::Value>, String) {
    let manifest_source = mpv_manifest_source(mpv_manifest);
    let files = required
        .iter()
        .map(|path| {
            let content = if *path == required[5] {
                manifest_source.as_bytes()
            } else {
                path.as_bytes()
            };
            write_release_file(root, path, content)
        })
        .collect::<Vec<_>>();
    let outer = outer_manifest(&files);
    (files, outer)
}

#[test]
fn release_and_custom_profiles_fail_closed_without_the_manifest() {
    use build_support::RuntimeResourceConfigAction;

    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), false, None),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), false, Some("{}")),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("custom"), false, Some("{}")),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), true, None),
        Ok(RuntimeResourceConfigAction::Keep),
    );
}

#[test]
fn debug_and_test_profiles_merge_empty_resources_into_the_existing_override() {
    use build_support::RuntimeResourceConfigAction;
    use serde_json::{json, Value};

    let assert_development_override = |input: Option<&str>, expected: Value| {
        let action = build_support::runtime_resource_config_action(Some("debug"), false, input)
            .expect("debug 缺清单必须能构造开发 override");
        let RuntimeResourceConfigAction::UseDevelopmentOverride(override_json) = action else {
            panic!("debug 缺清单必须返回合并后的开发 override");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&override_json).expect("override 必须是 JSON"),
            expected,
        );
    };

    assert_development_override(None, json!({ "bundle": { "resources": [] } }));
    assert_development_override(
        Some(r#"{"productName":"test"}"#),
        json!({ "productName": "test", "bundle": { "resources": [] } }),
    );
    assert_development_override(
        Some(r#"{"bundle":{"active":false}}"#),
        json!({ "bundle": { "active": false, "resources": [] } }),
    );
    let test_action = build_support::runtime_resource_config_action(Some("test"), false, None)
        .expect("test 缺清单必须能构造开发 override");
    assert!(matches!(
        test_action,
        RuntimeResourceConfigAction::UseDevelopmentOverride(_)
    ));

    for invalid_override in ["{", "[]", r#"{"bundle":null}"#] {
        assert!(
            build_support::runtime_resource_config_action(
                Some("debug"),
                false,
                Some(invalid_override),
            )
            .is_err(),
            "{invalid_override} 不能替换 Tauri 配置根对象或 bundle",
        );
    }

    assert_eq!(
        build_support::runtime_resource_config_action(
            Some("debug"),
            true,
            Some("{\"productName\":\"test\"}")
        ),
        Ok(RuntimeResourceConfigAction::Keep),
    );
}

#[test]
fn release_config_allows_unrelated_override_but_rejects_resource_changes() {
    let base_config = include_str!("../tauri.conf.json");

    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), true, Some("{}")),
        Ok(build_support::RuntimeResourceConfigAction::Keep),
    );
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"productName":"test build"}"#),
    )
    .is_ok());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":["runtime-resources.json","embedded-runtime-resources"]}}"#),
    )
    .is_ok());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":[]}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":null}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":null}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":["runtime-resources.json","extra.json"]}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":{"bundle":{"resources":["runtime-resources.json"]}}}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        r#"{"bundle":{"resources":[]}}"#,
        None,
    )
    .is_err());
}

#[test]
fn windows_release_tree_requires_schema_v2_and_fixed_eleven_files() {
    let root = std::env::temp_dir().join(format!(
        "autolive-release-resource-contract-{}",
        std::process::id()
    ));
    let _ignored = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create release root");
    let required = [
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
    let report_hash = sha256(b"phase7a report");
    let mpv_manifest = serde_json::json!({
        "schema_version": 2,
        "target": "x86_64-pc-windows-msvc",
        "build": {
            "scope": "phase7a_supply_candidate",
            "claim": "one_locked_cold_build_candidate",
            "lock_sha256": sha256(b"phase7a lock"),
            "report_sha256": report_hash.clone(),
        },
        "components": {
            "mpv": {
                "source_ref": "7b8915bc1d04c7e1b61184e00c7fbfaab1911e75",
                "license_expression": "GPL-2.0-or-later",
            },
            "libplacebo": {
                "source_ref": "22ee762e8e0890fc54068beb670310f0edce7263",
                "license_expression": "LGPL-2.1-or-later",
            },
            "ffmpeg": {
                "source_ref": "1d7b14f61d66fdf18f15204c613df9d65396c319",
                "license_expression": "GPL-2.0-or-later",
            },
        },
        "audit": {
            "release_review_status": "approved",
            "corresponding_source_complete": true,
            "third_party_notices_reviewed": true,
        },
        "files": {
            "mpv.exe": manifest_descriptor(required[2].as_bytes()),
            "spirv-cross-c-shared.dll": manifest_descriptor(required[3].as_bytes()),
            "vulkan-1.dll": manifest_descriptor(required[4].as_bytes()),
            "legal/Copyright.txt": manifest_descriptor(required[6].as_bytes()),
            "legal/GPL-2.0.txt": manifest_descriptor(required[7].as_bytes()),
            "legal/LGPL-2.1.txt": manifest_descriptor(required[8].as_bytes()),
            "legal/SOURCE.md": manifest_descriptor(required[9].as_bytes()),
            "legal/THIRD-PARTY-NOTICES.md": manifest_descriptor(required[10].as_bytes()),
        },
        "supply_evidence": {
            "report": {
                "path": "reproducible-build-report.json",
                "size_bytes": 1,
                "sha256": report_hash,
            },
            "corresponding_source": {
                "path": "build-evidence/corresponding-source.tar.zst",
                "size_bytes": 2,
                "sha256": sha256(b"corresponding source"),
            },
            "cyclonedx_sbom": {
                "path": "build-evidence/sbom.cdx.json",
                "size_bytes": 3,
                "sha256": sha256(b"cyclonedx"),
            },
            "spdx_sbom": {
                "path": "build-evidence/sbom.spdx.json",
                "size_bytes": 4,
                "sha256": sha256(b"spdx"),
            },
            "license_inventory": {
                "path": "build-evidence/license-inventory.txt",
                "size_bytes": 5,
                "sha256": sha256(b"licenses"),
            },
            "copyright_inventory": {
                "path": "build-evidence/copyright-inventory.txt",
                "size_bytes": 6,
                "sha256": sha256(b"copyrights"),
            },
        },
    });
    let (_, manifest) = write_windows_release_tree(&root, &required, &mpv_manifest);

    assert!(validate_windows_release_tree(&manifest, &root, &mpv_manifest).is_ok());
    assert!(build_support::validate_release_runtime_resource_tree(
        &manifest,
        &root,
        "aarch64-apple-darwin",
        None,
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_tree(
        r#"{"schema_version":1,"target":"aarch64-apple-darwin","files":[]}"#,
        &root,
        "aarch64-apple-darwin",
        None,
    )
    .is_ok());

    let mut outer_schema_v2: serde_json::Value =
        serde_json::from_str(&manifest).expect("parse outer manifest");
    outer_schema_v2["schema_version"] = serde_json::json!(2);
    assert!(validate_windows_release_tree(
        &serde_json::to_string(&outer_schema_v2).expect("outer schema JSON"),
        &root,
        &mpv_manifest,
    )
    .is_err());

    let mut blocked = mpv_manifest.clone();
    blocked["audit"] = serde_json::json!({
        "release_review_status": "blocked",
        "corresponding_source_complete": false,
        "third_party_notices_reviewed": false,
    });
    let (_, blocked_outer) = write_windows_release_tree(&root, &required, &blocked);
    assert!(validate_windows_release_tree(&blocked_outer, &root, &blocked).is_err());

    let mut short_commit = mpv_manifest.clone();
    short_commit["components"]["mpv"]["source_ref"] = serde_json::json!("7b8915bc1d");
    let (_, short_commit_outer) = write_windows_release_tree(&root, &required, &short_commit);
    assert!(validate_windows_release_tree(&short_commit_outer, &root, &short_commit).is_err());

    let mut missing_ffmpeg = mpv_manifest.clone();
    missing_ffmpeg["components"]
        .as_object_mut()
        .expect("components")
        .remove("ffmpeg");
    let (_, missing_ffmpeg_outer) = write_windows_release_tree(&root, &required, &missing_ffmpeg);
    assert!(validate_windows_release_tree(&missing_ffmpeg_outer, &root, &missing_ffmpeg).is_err());

    let mut extra_component = mpv_manifest.clone();
    extra_component["components"]["shaderc"] = serde_json::json!({
        "source_ref": "7060a6615a1c6e2515e696651eea685524ecadb5",
        "license_expression": "Apache-2.0",
    });
    let (_, extra_component_outer) = write_windows_release_tree(&root, &required, &extra_component);
    assert_eq!(
        validate_windows_release_tree(&extra_component_outer, &root, &extra_component)
            .expect_err("额外自签组件必须被精确组件集合门禁拒绝"),
        "Windows mpv 发布清单对象字段不匹配：components",
    );

    let mut wrong_claim = mpv_manifest.clone();
    wrong_claim["build"]["claim"] = serde_json::json!("self_asserted");
    let (_, wrong_claim_outer) = write_windows_release_tree(&root, &required, &wrong_claim);
    assert!(validate_windows_release_tree(&wrong_claim_outer, &root, &wrong_claim).is_err());

    let mut wrong_report_hash = mpv_manifest.clone();
    wrong_report_hash["supply_evidence"]["report"]["sha256"] =
        serde_json::json!(sha256(b"different report"));
    let (_, wrong_report_outer) = write_windows_release_tree(&root, &required, &wrong_report_hash);
    assert!(validate_windows_release_tree(&wrong_report_outer, &root, &wrong_report_hash).is_err());

    let mut unsafe_evidence_path = mpv_manifest.clone();
    unsafe_evidence_path["supply_evidence"]["spdx_sbom"]["path"] =
        serde_json::json!("../sbom.spdx.json");
    let (_, unsafe_evidence_outer) =
        write_windows_release_tree(&root, &required, &unsafe_evidence_path);
    assert!(
        validate_windows_release_tree(&unsafe_evidence_outer, &root, &unsafe_evidence_path,)
            .is_err()
    );

    let mut wrong_evidence_role_path = mpv_manifest.clone();
    wrong_evidence_role_path["supply_evidence"]["spdx_sbom"]["path"] =
        serde_json::json!("build-evidence/other.spdx.json");
    let (_, wrong_evidence_role_outer) =
        write_windows_release_tree(&root, &required, &wrong_evidence_role_path);
    assert!(validate_windows_release_tree(
        &wrong_evidence_role_outer,
        &root,
        &wrong_evidence_role_path,
    )
    .is_err());

    let mut extra_root = mpv_manifest.clone();
    extra_root["artifact"] = serde_json::json!({ "legacy": true });
    let (_, extra_root_outer) = write_windows_release_tree(&root, &required, &extra_root);
    assert!(validate_windows_release_tree(&extra_root_outer, &root, &extra_root).is_err());

    let source_reference = mpv_manifest_source(&mpv_manifest);
    let replacement_contents = required
        .iter()
        .map(|path| format!("replacement:{path}").into_bytes())
        .collect::<Vec<_>>();
    let mut replaced_manifest = mpv_manifest.clone();
    for (manifest_name, required_index) in [
        ("mpv.exe", 2),
        ("spirv-cross-c-shared.dll", 3),
        ("vulkan-1.dll", 4),
        ("legal/Copyright.txt", 6),
        ("legal/GPL-2.0.txt", 7),
        ("legal/LGPL-2.1.txt", 8),
        ("legal/SOURCE.md", 9),
        ("legal/THIRD-PARTY-NOTICES.md", 10),
    ] {
        replaced_manifest["files"][manifest_name] =
            manifest_descriptor(&replacement_contents[required_index]);
    }
    let replaced_manifest_source = mpv_manifest_source(&replaced_manifest);
    let replaced_files = required
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let content = if index == 5 {
                replaced_manifest_source.as_bytes()
            } else {
                replacement_contents[index].as_slice()
            };
            write_release_file(&root, path, content)
        })
        .collect::<Vec<_>>();
    let replaced_outer = outer_manifest(&replaced_files);
    assert!(build_support::validate_release_runtime_resource_tree(
        &replaced_outer,
        &root,
        "x86_64-pc-windows-msvc",
        Some(&replaced_manifest_source),
    )
    .is_ok());
    assert_eq!(
        build_support::validate_release_runtime_resource_tree(
            &replaced_outer,
            &root,
            "x86_64-pc-windows-msvc",
            Some(&source_reference),
        )
        .expect_err("同时替换清单、资源和外层哈希仍必须被源码侧字节锚点拒绝"),
        "Windows mpv 包内发布清单与源码侧候选清单字节不一致",
    );

    let (mut files, _) = write_windows_release_tree(&root, &required, &mpv_manifest);
    files[9] = write_release_file(&root, required[9], b"tampered but outer hash updated");
    assert!(validate_windows_release_tree(&outer_manifest(&files), &root, &mpv_manifest,).is_err());

    let (mut files, _) = write_windows_release_tree(&root, &required, &mpv_manifest);
    fs::remove_file(release_path(&root, required[6])).expect("remove legal file");
    assert!(validate_windows_release_tree(&outer_manifest(&files), &root, &mpv_manifest,).is_err());
    fs::write(release_path(&root, required[6]), required[6].as_bytes())
        .expect("restore legal file");
    files[6] = write_release_file(&root, required[6], required[6].as_bytes());
    files[6]["executable"] = serde_json::json!(true);
    assert!(validate_windows_release_tree(&outer_manifest(&files), &root, &mpv_manifest,).is_err());

    let (files, manifest) = write_windows_release_tree(&root, &required, &mpv_manifest);
    let rogue_path = "x86_64-pc-windows-msvc/binaries/undeclared.dll";
    write_release_file(&root, rogue_path, b"rogue");
    assert!(validate_windows_release_tree(&manifest, &root, &mpv_manifest).is_err());
    fs::remove_file(release_path(&root, rogue_path)).expect("remove rogue");
    let mut declared_rogue = files;
    declared_rogue.push(serde_json::json!({
        "relative_path": rogue_path,
        "sha256": sha256(b"rogue"),
        "size_bytes": 5,
        "executable": true,
        "component": "media",
    }));
    assert!(
        validate_windows_release_tree(&outer_manifest(&declared_rogue), &root, &mpv_manifest,)
            .is_err()
    );

    let _ignored = fs::remove_dir_all(root);
}

#[test]
fn rust_release_gate_does_not_pin_legacy_binary_urls_or_short_commits() {
    let source = include_str!("../build_support.rs");
    assert!(!source.contains("shinchiro/mpv-winbuild-cmake"));
    assert!(!source.contains("20260814"));
    assert!(!source.contains("7b8915bc1d\""));
}

#[test]
fn release_build_script_validates_the_embedded_resource_tree() {
    let build_script = include_str!("../build.rs");
    assert!(build_script.contains("validate_release_runtime_resource_tree"));
    assert!(build_script.contains("include_str!("));
    assert!(build_script
        .contains("../third_party/mpv/x86_64-pc-windows-msvc/legal/mpv-runtime-manifest.json"));
}

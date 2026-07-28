// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Generates the compile-time custom-indicator registry from `src/custom/*.rs`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn valid_module_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && name.as_bytes()[0].is_ascii_lowercase()
}

fn escaped_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let custom_dir = manifest_dir.join("src/custom");
    println!("cargo:rerun-if-changed={}", custom_dir.display());

    let mut modules = Vec::new();
    if custom_dir.exists() {
        for entry in fs::read_dir(&custom_dir).expect("read custom-indicator directory") {
            let entry = entry.expect("read custom-indicator entry");
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("UTF-8 filename");
            if !valid_module_name(name) {
                panic!("custom indicator filename `{name}` must be lowercase ASCII snake_case");
            }
            modules.push((name.to_owned(), path));
        }
    }
    modules.sort_by(|left, right| left.0.cmp(&right.0));

    let mut source = String::from("use super::CustomIndicatorRegistration;\n");
    for (name, path) in &modules {
        source.push_str(&format!(
            "#[path = \"{}\"] mod {};\n",
            escaped_path(path),
            name
        ));
    }
    source.push_str("static REGISTRATIONS: &[CustomIndicatorRegistration] = &[\n");
    for (name, _) in &modules {
        source.push_str(&format!("    {}::REGISTRATION,\n", name));
    }
    source.push_str("];\npub(super) fn registrations() -> &'static [CustomIndicatorRegistration] { REGISTRATIONS }\n");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("custom_indicator_registry.rs"), source).expect("write custom registry");
}

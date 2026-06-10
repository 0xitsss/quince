// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
use std::fs;
use std::path::Path;
mod extract;
use extract::{extract_items, extract_module_doc, render_items_section};

const BOOK_SRC: &str = "book/src";
const API_PREFIX: &str = "api";

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let src_dir = root.join(BOOK_SRC);
    let api_dir = src_dir.join(API_PREFIX);

    // Clean and recreate api dir
    let _ = fs::remove_dir_all(&api_dir);
    fs::create_dir_all(&api_dir).expect("create api dir");

    let crates = [
        ("qfl/src", "qfl"),
        ("core/src", "core"),
        ("engine/src", "engine"),
        ("exchange/src", "exchange"),
        ("indicators/src", "indicators"),
        ("logger/src", "logger"),
        ("risk/src", "risk"),
        ("quince/src", "quince"),
    ];

    // Crate-level entries: we collect them for SUMMARY.md generation
    let mut per_crate: Vec<(&str, Vec<(String, String)>)> = Vec::new();

    for (src_rel, crate_name) in &crates {
        let crate_src = root.join(src_rel);
        if !crate_src.exists() {
            continue;
        }
        let crate_api = api_dir.join(crate_name);
        fs::create_dir_all(&crate_api).expect("create crate api dir");

        let mut entries: Vec<(String, String)> = Vec::new();
        collect_files(&crate_src, &crate_src, "", &crate_api, &mut entries);

        if entries.is_empty() {
            continue;
        }

        // Write crate-level SUMMARY.md
        let mut crate_summary = String::new();
        crate_summary.push_str(&format!("# {}\n\n", crate_name));
        for (display_name, file_rel) in &entries {
            let md_rel = file_rel.replace('\\', "/");
            crate_summary.push_str(&format!("- [{}]({}.md)\n", display_name, md_rel));
        }
        fs::write(crate_api.join("SUMMARY.md"), &crate_summary).expect("write crate summary");

        per_crate.push((crate_name, entries));
    }

    // Update main SUMMARY.md with API links
    let summary_path = src_dir.join("SUMMARY.md");
    let summary_content = fs::read_to_string(&summary_path).expect("read SUMMARY.md");

    let marker = "--DOCGEN:API--";
    if let Some(pos) = summary_content.find(marker) {
        let header = &summary_content[..pos + marker.len()];
        let mut new_summary = String::from(header);
        new_summary.push('\n');

        for (crate_name, entries) in &per_crate {
            new_summary.push_str(&format!("\n### {}\n\n", crate_name));
            for (display_name, file_rel) in entries {
                let md_rel = file_rel.replace('\\', "/");
                new_summary.push_str(&format!(
                    "- [{}]({}/{}/{}.md)\n",
                    display_name, API_PREFIX, crate_name, md_rel
                ));
            }
        }

        fs::write(&summary_path, &new_summary).expect("write SUMMARY.md");
        println!(
            "docgen: {} modules in {} crates в†’ book/src/",
            per_crate.iter().map(|(_, e)| e.len()).sum::<usize>(),
            per_crate.len()
        );
    } else {
        eprintln!("docgen ERROR: marker '{}' not found in SUMMARY.md", marker);
    }
}

fn collect_files(
    base: &Path,
    dir: &Path,
    prefix: &str,
    crate_api: &Path,
    entries: &mut Vec<(String, String)>,
) {
    let mut files: Vec<_> = fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            e.file_type().map(|t| t.is_file()).unwrap_or(false) && name_str.ends_with(".rs")
        })
        .collect();
    files.sort_by_key(|e| e.file_name());

    for entry in files {
        let path = entry.path();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();

        let file_rel = if stem == "lib" || stem == "main" {
            if prefix.is_empty() {
                if stem == "main" {
                    "main".into()
                } else {
                    "index".into()
                }
            } else {
                format!("{}/index", prefix)
            }
        } else {
            if prefix.is_empty() {
                stem.clone()
            } else {
                format!("{}/{}", prefix, stem)
            }
        };

        let display_name = if stem == "lib" {
            if prefix.is_empty() {
                "lib".into()
            } else {
                format!("{}/lib", prefix)
            }
        } else if stem == "main" {
            if prefix.is_empty() {
                "main".into()
            } else {
                format!("{}/main", prefix)
            }
        } else {
            if prefix.is_empty() {
                stem.clone()
            } else {
                format!("{}/{}", prefix, stem)
            }
        };

        let content = fs::read_to_string(&path).expect("read file");
        let doc = extract_module_doc(&content);
        let rel_src = pathdiff(base, &path);

        let items = extract_items(&content);
        let mut md_content = String::new();

        if let Some(ref text) = doc {
            md_content.push_str(&format!(
                "# Module: `{}`\n\n> Source: `{}`\n\n{}\n",
                display_name, rel_src, text
            ));
        } else {
            md_content.push_str(&format!(
                "# Module: `{}`\n\n> Source: `{}`\n\n_No module documentation._\n",
                display_name, rel_src
            ));
        }

        for (category, items) in &items {
            md_content.push_str(&render_items_section(category, items));
        }

        let md_rel = file_rel.clone() + ".md";
        let md_path = crate_api.join(&md_rel);
        if let Some(parent) = md_path.parent() {
            fs::create_dir_all(parent).expect("create md parent");
        }
        fs::write(&md_path, &md_content).expect("write md");

        entries.push((display_name, file_rel));
    }

    let mut subdirs: Vec<_> = fs::read_dir(dir)
        .expect("read dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false) && e.file_name() != "benches"
        })
        .collect();
    subdirs.sort_by_key(|e| e.file_name());

    for entry in subdirs {
        let name = entry.file_name().to_string_lossy().to_string();
        let new_prefix = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };
        collect_files(base, &entry.path(), &new_prefix, crate_api, entries);
    }
}

fn pathdiff(base: &Path, file: &Path) -> String {
    let base_str = base.to_string_lossy().replace('\\', "/");
    let file_str = file.to_string_lossy().replace('\\', "/");
    if let Some(rest) = file_str.strip_prefix(&base_str) {
        rest.trim_start_matches('/').to_string()
    } else {
        file_str
    }
}

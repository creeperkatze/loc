use std::collections::HashMap;
use std::io::Read;

use flate2::read::GzDecoder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tar::Archive;

use crate::error::AppError;

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum LocsChild {
    File(u64),
    Dir(Locs),
}

#[derive(Serialize, Deserialize, Default)]
pub struct Locs {
    pub loc: u64,
    #[serde(rename = "locByLangs", skip_serializing_if = "Option::is_none")]
    pub loc_by_langs: Option<HashMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<HashMap<String, LocsChild>>,
}

// Mutable tree used while walking the tarball, before totals are aggregated.
enum BuildNode {
    File { loc: u64, lang: String },
    Dir(HashMap<String, BuildNode>),
}

// Downloads and walks a repo's tarball, counting lines of code per file, aggregated into a directory tree broken down by extension.
pub fn compute_locs(tarball: &[u8], filters: &[Regex]) -> Result<Locs, AppError> {
    let decoder = GzDecoder::new(tarball);
    let mut archive = Archive::new(decoder);

    let mut root: HashMap<String, BuildNode> = HashMap::new();

    let entries = archive
        .entries()
        .map_err(|e| AppError::Upstream(format!("invalid archive: {e}")))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| AppError::Upstream(format!("invalid archive entry: {e}")))?;

        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }

        let path = entry
            .path()
            .map_err(|e| AppError::Upstream(e.to_string()))?
            .into_owned();

        let mut components: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();

        if components.is_empty() {
            continue;
        }
        // Drop the tarball's synthetic root directory (e.g. "owner-repo-sha/").
        components.remove(0);
        if components.is_empty() {
            continue;
        }

        let filename = components.last().unwrap().clone();
        let lang = extension_key(&filename);

        if !filters.is_empty() && !filters.iter().any(|re| re.is_match(&lang)) {
            continue;
        }

        let mut content = Vec::new();
        entry
            .read_to_end(&mut content)
            .map_err(|e| AppError::Upstream(e.to_string()))?;

        if is_binary(&content) {
            continue;
        }

        let loc = count_lines(&content);

        insert(&mut root, &components, loc, lang);
    }

    Ok(finalize(root))
}

fn insert(root: &mut HashMap<String, BuildNode>, components: &[String], loc: u64, lang: String) {
    let mut current = root;

    for dir in &components[..components.len() - 1] {
        let node = current
            .entry(dir.clone())
            .or_insert_with(|| BuildNode::Dir(HashMap::new()));

        if !matches!(node, BuildNode::Dir(_)) {
            *node = BuildNode::Dir(HashMap::new());
        }

        current = match node {
            BuildNode::Dir(children) => children,
            BuildNode::File { .. } => unreachable!(),
        };
    }

    let filename = components.last().unwrap().clone();
    current.insert(filename, BuildNode::File { loc, lang });
}

fn finalize(children: HashMap<String, BuildNode>) -> Locs {
    let mut total_loc = 0u64;
    let mut loc_by_langs: HashMap<String, u64> = HashMap::new();
    let mut out_children: HashMap<String, LocsChild> = HashMap::new();

    for (name, node) in children {
        match node {
            BuildNode::File { loc, lang } => {
                total_loc += loc;
                *loc_by_langs.entry(lang).or_insert(0) += loc;
                out_children.insert(name, LocsChild::File(loc));
            }
            BuildNode::Dir(dir_children) => {
                let dir_locs = finalize(dir_children);
                total_loc += dir_locs.loc;
                if let Some(langs) = &dir_locs.loc_by_langs {
                    for (lang, loc) in langs {
                        *loc_by_langs.entry(lang.clone()).or_insert(0) += loc;
                    }
                }
                out_children.insert(name, LocsChild::Dir(dir_locs));
            }
        }
    }

    Locs {
        loc: total_loc,
        loc_by_langs: Some(loc_by_langs),
        children: Some(out_children),
    }
}

// Extracts the "language key" for a file: its lowercased extension (with leading dot), or the whole filename for extension-less/dotfiles (e.g. "Dockerfile", ".gitignore"), matching ghloc's convention.
fn extension_key(filename: &str) -> String {
    match filename.rfind('.') {
        Some(pos) if pos > 0 => filename[pos..].to_lowercase(),
        _ => filename.to_string(),
    }
}

// Simple binary-file heuristic: a NUL byte anywhere in the first few KB.
fn is_binary(content: &[u8]) -> bool {
    let sample_len = content.len().min(8000);
    content[..sample_len].contains(&0)
}

// Counts lines like `str::lines()` would, but scanning raw bytes instead of paying for a UTF-8 validating copy of the whole file.
fn count_lines(content: &[u8]) -> u64 {
    if content.is_empty() {
        return 0;
    }

    let newlines = content.iter().filter(|&&b| b == b'\n').count() as u64;
    if content[content.len() - 1] == b'\n' {
        newlines
    } else {
        newlines + 1
    }
}

pub fn parse_filters(filter: Option<&str>) -> Result<Vec<Regex>, AppError> {
    let Some(raw) = filter else {
        return Ok(Vec::new());
    };

    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|pattern| {
            Regex::new(pattern)
                .map_err(|e| AppError::BadRequest(format!("invalid filter regex '{pattern}': {e}")))
        })
        .collect()
}

pub fn humanize(value: u64) -> String {
    let mut value = value as f64;
    let mut suffix = "";

    for suff in ["", "k", "M"] {
        if value < 1000.0 {
            suffix = suff;
            break;
        }
        value /= 1000.0;
    }

    let precision = if value < 50.0 { 1 } else { 0 };
    let formatted = format!("{value:.precision$}");
    let trimmed = if formatted.contains('.') {
        formatted.trim_end_matches('0').trim_end_matches('.')
    } else {
        &formatted
    };

    format!("{trimmed}{suffix}")
}

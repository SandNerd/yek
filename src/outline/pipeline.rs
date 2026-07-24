//! Apply outline rendering to processed files according to the configured mode.
//!
//! Runs after files are read and before the budget step in `concat_files`.

use std::str::FromStr;

use bytesize::ByteSize;
use rayon::prelude::*;

use super::{detect_language, extract, render, Language, OutlineLevel};
use crate::config::{OutlineFallback, OutlineLevel as CfgLevel, OutlineMode, YekConfig};
use crate::models::ProcessedFile;

/// Transform `files` in place according to `config`'s outline mode.
pub fn apply(files: &mut Vec<ProcessedFile>, config: &YekConfig) {
    let level = cfg_to_level(config.outline_level());
    let tag = tag(config.outline_level());
    let restrict = &config.outline_languages;

    match config.outline_mode() {
        OutlineMode::Off => {}
        OutlineMode::Always => {
            files.par_iter_mut().for_each(|f| {
                if let Some(rendered) = outline_one(&f.rel_path, &f.content, restrict, level) {
                    f.set_content(display(config, tag, &rendered));
                    f.outline_level = Some(tag);
                }
            });
            if config.outline_fallback() == OutlineFallback::Omit {
                files.retain(|f| f.outline_level.is_some());
            }
        }
        OutlineMode::Degrade => degrade(files, config, level, tag),
    }
}

/// Budget-aware Suffix-Floor allocator: pay the minimum outline (or full, if
/// outlining is unsupported) cost for every file first, then greedily upgrade
/// high-priority outlineable files to full content from the leftover
/// discretionary budget. The final hard cap is still enforced by `concat_files`.
fn degrade(
    files: &mut [ProcessedFile],
    config: &YekConfig,
    level: OutlineLevel,
    tag: &'static str,
) {
    // Pre-render outlines in parallel; only supported files yield `Some`.
    let outlines: Vec<Option<String>> = files
        .par_iter()
        .map(|f| outline_one(&f.rel_path, &f.content, &config.outline_languages, level))
        .collect();

    // Pass 1: floor cost — outline size when supported, else full size.
    let mut total_floor_cost = 0usize;
    let mut metrics: Vec<(usize, usize)> = Vec::with_capacity(files.len());
    for (i, file) in files.iter().enumerate() {
        let full_size = cost(config, &file.content);
        let outline_size = match &outlines[i] {
            Some(rendered) => {
                let rendered_size = cost(config, &display(config, tag, rendered));
                std::cmp::min(full_size, rendered_size)
            }
            None => full_size,
        };
        total_floor_cost += outline_size;
        metrics.push((full_size, outline_size));
    }

    let max_tokens = budget(config);

    // Even the skeleton won't fit: outline everything we can and let
    // `concat_files` drop the low-priority tail under its hard cap.
    if max_tokens < total_floor_cost {
        for (i, file) in files.iter_mut().enumerate() {
            if let Some(rendered) = &outlines[i] {
                file.set_content(display(config, tag, rendered));
                file.outline_level = Some(tag);
            }
        }
        return;
    }

    // Priority-descending order (importance first, path as tie-break).
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by(|&a, &b| {
        files[b]
            .priority
            .cmp(&files[a].priority)
            .then_with(|| files[a].rel_path.cmp(&files[b].rel_path))
    });

    // Pass 2: spend discretionary budget upgrading outline → full, highest first.
    // Do not break early — a later, smaller file may still fit.
    let mut discretionary_budget = max_tokens.saturating_sub(total_floor_cost);
    for i in order {
        let Some(rendered) = &outlines[i] else {
            // Unsupported: already Full; its full cost is in the floor.
            continue;
        };
        let (full_size, outline_size) = metrics[i];
        let upgrade_cost = full_size.saturating_sub(outline_size);
        if discretionary_budget >= upgrade_cost {
            discretionary_budget -= upgrade_cost;
            // Keep full content (no mutation).
        } else {
            files[i].set_content(display(config, tag, rendered));
            files[i].outline_level = Some(tag);
        }
    }
}

/// Render `content` as an outline, or `None` if the language is unsupported (or
/// excluded by `restrict`), the parse is unusable, or there is nothing to show.
fn outline_one(
    rel_path: &str,
    content: &str,
    restrict: &[String],
    level: OutlineLevel,
) -> Option<String> {
    let lang = detect_language(rel_path)?;
    if !restrict.is_empty()
        && !restrict
            .iter()
            .any(|r| Language::from_name(r) == Some(lang))
    {
        return None;
    }
    let symbols = extract(content, lang)?;
    Some(render(content, lang, &symbols, level))
}

/// Text output marks outlined files so a reader knows the content is abbreviated;
/// JSON output omits the marker and relies on the structured `level` field.
fn display(config: &YekConfig, tag: &str, rendered: &str) -> String {
    if config.json {
        rendered.to_string()
    } else {
        format!("⟪yek:{tag}⟫\n{rendered}")
    }
}

fn budget(config: &YekConfig) -> usize {
    if config.token_mode {
        crate::parse_token_limit(&config.tokens).unwrap_or(usize::MAX)
    } else {
        ByteSize::from_str(&config.max_size)
            .map(|b| b.as_u64() as usize)
            .unwrap_or(usize::MAX)
    }
}

/// Approximate per-file cost. Counts raw content only, not the template or JSON
/// wrapper that `concat_files` adds, so it slightly under-counts. The
/// discretionary leftover plus `concat_files`' exact final cap keep this safe.
fn cost(config: &YekConfig, content: &str) -> usize {
    if config.token_mode {
        crate::count_tokens(content)
    } else {
        content.len()
    }
}

fn cfg_to_level(level: CfgLevel) -> OutlineLevel {
    match level {
        CfgLevel::Outline => OutlineLevel::Outline,
        CfgLevel::Api => OutlineLevel::Api,
        CfgLevel::Symbols => OutlineLevel::Symbols,
    }
}

fn tag(level: CfgLevel) -> &'static str {
    match level {
        CfgLevel::Outline => "outline",
        CfgLevel::Api => "api",
        CfgLevel::Symbols => "symbols",
    }
}

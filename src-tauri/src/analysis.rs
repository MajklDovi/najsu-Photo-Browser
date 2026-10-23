//! Local (offline) photo quality analysis.
//!
//! Every metric here is computed from pixels and EXIF on this machine - nothing is
//! uploaded. Each category yields a 0-5 score; combining them into one rating (weights,
//! caps, series penalties) happens in the frontend so it can be tuned live without
//! re-analysing. Categories that need a vision model (composition, faces, subject,
//! overall impact) are deliberately absent for now; `ai_json` is reserved for them.

use crate::exiftool;
use image::{imageops, DynamicImage};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use tauri::{AppHandle, Emitter};

/// Bump when the algorithms change so stale results get recomputed.
pub const ANALYSIS_VERSION: i64 = 1;
const ANALYSIS_EDGE: u32 = 1024;
const GRID: usize = 8;

#[derive(Serialize, Clone, Debug)]
pub struct AnalysisRecord {
    pub path: String,
    pub sharpness: f64,
    pub shake: f64,
    pub exposure: f64,
    pub noise: f64,
    /// None when the photo has no clear horizon / vertical lines to measure.
    pub horizon: Option<f64>,
    pub horizon_deg: Option<f64>,
    pub series_id: Option<String>,
    /// Result of the optional local AI review (composition, subject, people, impact).
    pub ai: Option<crate::ai::AiResult>,
}

#[derive(Debug, Clone)]
pub struct Metrics {
    pub sharpness: f64,
    pub shake: f64,
    pub exposure: f64,
    pub noise: f64,
    pub horizon: Option<f64>,
    pub horizon_deg: Option<f64>,
    pub sharp_region: f64,
    pub dhash: u64,
    /// Raw (noise-corrected) edge strength behind `sharpness`; kept for calibration.
    #[allow(dead_code)]
    pub edge_strength: f64,
    /// Axis imbalance of edge energy in the sharpest tiles (1 = isotropic); calibration only.
    #[allow(dead_code)]
    pub axis_ratio: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ExifInfo {
    pub capture_time: Option<f64>,
    pub exposure_time: Option<f64>,
    pub focal35: Option<f64>,
}

pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS analysis (
            path TEXT PRIMARY KEY,
            version INTEGER NOT NULL,
            analyzed_at INTEGER NOT NULL,
            sharpness REAL NOT NULL,
            shake REAL NOT NULL,
            exposure REAL NOT NULL,
            noise REAL NOT NULL,
            horizon REAL,
            horizon_deg REAL,
            sharp_region REAL,
            capture_time REAL,
            dhash INTEGER,
            series_id TEXT,
            ai_json TEXT
        );
        CREATE TABLE IF NOT EXISTS series_ai (
            series_id TEXT PRIMARY KEY,
            members TEXT NOT NULL,
            points TEXT NOT NULL,
            reason TEXT NOT NULL,
            model TEXT NOT NULL,
            passes INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        );",
    )
}

/// Keeps the catalogue in step with a rating that was just written to disk.
pub fn record_rating(app: &AppHandle, folder: &str, path: &str, rating: i32) {
    if let Ok(conn) = crate::catalog::open_db(app, folder) {
        let _ = conn.execute(
            "UPDATE photos SET rating = ?1, rated = 1 WHERE path = ?2",
            params![rating, path],
        );
    }
}

// ---------------------------------------------------------------------------
// Pixel analysis
// ---------------------------------------------------------------------------

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// Mean of the strongest `frac` of the values (sorts a copy; fine for tile-sized input).
fn top_fraction_mean(values: &mut [f32], frac: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let n = values.len();
    let keep = ((n as f32 * frac).ceil() as usize).clamp(1, n);
    let pivot = n - keep;
    values.select_nth_unstable_by(pivot, |a, b| a.partial_cmp(b).unwrap());
    values[pivot..].iter().sum::<f32>() / keep as f32
}

struct Gradients {
    w: usize,
    h: usize,
    gx: Vec<f32>,
    gy: Vec<f32>,
    mag: Vec<f32>,
}

fn sobel(px: &[u8], w: usize, h: usize) -> Gradients {
    let mut gx = vec![0f32; w * h];
    let mut gy = vec![0f32; w * h];
    let mut mag = vec![0f32; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let a = px[i - w - 1] as i32;
            let b = px[i - w] as i32;
            let c = px[i - w + 1] as i32;
            let d = px[i - 1] as i32;
            let f = px[i + 1] as i32;
            let g = px[i + w - 1] as i32;
            let hh = px[i + w] as i32;
            let k = px[i + w + 1] as i32;
            let sx = ((c + 2 * f + k) - (a + 2 * d + g)) as f32 / 4.0;
            let sy = ((g + 2 * hh + k) - (a + 2 * b + c)) as f32 / 4.0;
            gx[i] = sx;
            gy[i] = sy;
            mag[i] = (sx * sx + sy * sy).sqrt();
        }
    }
    Gradients { w, h, gx, gy, mag }
}

struct Tile {
    /// Mean gradient magnitude of the tile's strongest 10% of pixels (edge strength).
    edge: f32,
    /// Mean gradient magnitude over the whole tile (texture / flatness).
    mean: f32,
    abs_gx: f32,
    abs_gy: f32,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

fn tiles(g: &Gradients) -> Vec<Tile> {
    let mut out = Vec::with_capacity(GRID * GRID);
    for ty in 0..GRID {
        for tx in 0..GRID {
            let x0 = (tx * g.w / GRID).max(1);
            let x1 = ((tx + 1) * g.w / GRID).min(g.w - 1);
            let y0 = (ty * g.h / GRID).max(1);
            let y1 = ((ty + 1) * g.h / GRID).min(g.h - 1);
            let mut mags = Vec::with_capacity((x1 - x0) * (y1 - y0));
            for y in y0..y1 {
                for x in x0..x1 {
                    mags.push(g.mag[y * g.w + x]);
                }
            }
            let mean = mags.iter().sum::<f32>() / mags.len().max(1) as f32;
            let mut sorted = mags.clone();
            let edge = top_fraction_mean(&mut sorted, 0.10);
            // Directional energy over this tile's edge pixels, for motion-blur detection.
            let cutoff = {
                let mut s = mags.clone();
                let n = s.len();
                if n == 0 {
                    0.0
                } else {
                    let idx = (n as f32 * 0.9) as usize;
                    s.select_nth_unstable_by(idx.min(n - 1), |a, b| a.partial_cmp(b).unwrap());
                    s[idx.min(n - 1)]
                }
            };
            let (mut sgx, mut sgy) = (0f32, 0f32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y * g.w + x;
                    if g.mag[i] >= cutoff && cutoff > 0.0 {
                        sgx += g.gx[i].abs();
                        sgy += g.gy[i].abs();
                    }
                }
            }
            out.push(Tile { edge, mean, abs_gx: sgx, abs_gy: sgy, x0, y0, x1, y1 });
        }
    }
    out
}

/// Luma spread of a tile (98th - 2nd percentile): the "height" of the edges inside it.
fn tile_range(px: &[u8], w: usize, t: &Tile) -> f32 {
    let mut hist = [0u32; 256];
    let mut n = 0u32;
    for y in t.y0..t.y1 {
        for x in t.x0..t.x1 {
            hist[px[y * w + x] as usize] += 1;
            n += 1;
        }
    }
    let pct = |p: f32| -> f32 {
        let target = (n as f32 * p) as u32;
        let mut acc = 0u32;
        for (i, &c) in hist.iter().enumerate() {
            acc += c;
            if acc >= target {
                return i as f32;
            }
        }
        255.0
    };
    pct(0.98) - pct(0.02)
}

/// Immerkaer's noise estimator over one tile (standard deviation, in 0-255 luma units).
fn tile_noise(px: &[u8], w: usize, t: &Tile) -> f64 {
    let (mut sum, mut n) = (0f64, 0f64);
    for y in t.y0.max(1)..t.y1.saturating_sub(1) {
        for x in t.x0.max(1)..t.x1.saturating_sub(1) {
            let i = y * w + x;
            let v = 4 * px[i] as i32
                - 2 * (px[i - 1] as i32 + px[i + 1] as i32 + px[i - w] as i32 + px[i + w] as i32)
                + (px[i - w - 1] as i32 + px[i - w + 1] as i32 + px[i + w - 1] as i32 + px[i + w + 1] as i32);
            sum += v.abs() as f64;
            n += 1.0;
        }
    }
    if n == 0.0 {
        return 0.0;
    }
    (std::f64::consts::PI / 2.0).sqrt() / 6.0 * (sum / n)
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Maps contrast-normalised edge strength (edge height / edge width, ~1/width in px at
/// 1024px) to 0-5. LO/HI come from calibration against progressively blurred real photos.
const SHARP_LO: f64 = 0.08;
const SHARP_HI: f64 = 0.50;

fn sharpness_score(normalized: f64) -> f64 {
    5.0 * clamp((normalized - SHARP_LO) / (SHARP_HI - SHARP_LO), 0.0, 1.0)
}

fn noise_score(sigma: f64) -> f64 {
    // ~1 luma level of noise at 1024px is clean; ~6 is very noisy.
    clamp(5.0 - (sigma - 1.0) / 5.0 * 5.0, 0.0, 5.0)
}

fn exposure_score(gray: &[u8]) -> f64 {
    let mut hist = [0u32; 256];
    for &p in gray {
        hist[p as usize] += 1;
    }
    let n = gray.len().max(1) as f64;
    let blown: u32 = hist[250..].iter().sum();
    let crushed: u32 = hist[..=5].iter().sum();
    let mean = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum::<f64>() / n;

    let percentile = |p: f64| -> f64 {
        let target = (n * p) as u32;
        let mut acc = 0u32;
        for (i, &c) in hist.iter().enumerate() {
            acc += c;
            if acc >= target {
                return i as f64;
            }
        }
        255.0
    };
    let range = percentile(0.99) - percentile(0.01);

    let mut penalty = 0.0;
    penalty += clamp(blown as f64 / n / 0.05, 0.0, 1.0) * 2.5;
    penalty += clamp(crushed as f64 / n / 0.10, 0.0, 1.0) * 2.0;
    penalty += clamp((60.0 - mean) / 40.0, 0.0, 1.0) * 2.0;
    penalty += clamp((mean - 190.0) / 40.0, 0.0, 1.0) * 2.0;
    penalty += clamp((100.0 - range) / 60.0, 0.0, 1.0) * 1.5;
    clamp(5.0 - penalty, 0.0, 5.0)
}

/// Looks for a dominant straight line direction close to horizontal (or, failing that,
/// vertical) among the strongest edges. Returns (tilt in degrees, confidence 0-1).
fn dominant_tilt(g: &Gradients, strong_thr: f32, vertical: bool, total_weight: f64) -> Option<(f64, f64)> {
    const RANGE: f64 = 10.0;
    const BIN: f64 = 0.25;
    let bins = (RANGE * 2.0 / BIN) as usize;
    let mut hist = vec![0f64; bins];

    for i in 0..g.mag.len() {
        let m = g.mag[i];
        if m < strong_thr {
            continue;
        }
        let theta = (g.gy[i] as f64).atan2(g.gx[i] as f64).to_degrees();
        // Horizontal edges have vertical gradients (theta = +-90); vertical edges the opposite.
        let dev = if vertical {
            (theta + 90.0).rem_euclid(180.0) - 90.0
        } else {
            theta.rem_euclid(180.0) - 90.0
        };
        if dev.abs() < RANGE {
            let b = (((dev + RANGE) / BIN) as usize).min(bins - 1);
            hist[b] += m as f64;
        }
    }

    // Sliding window of 10 bins (~2.5 degrees): wide enough that per-pixel noise in the
    // edge direction doesn't make the peak jump around.
    let win = 10;
    let sums: Vec<f64> = (0..=bins - win).map(|s| hist[s..s + win].iter().sum()).collect();
    let (best_i, &best) = sums
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())?;
    let mut sorted = sums.clone();
    let med = median(&mut sorted);
    let dominance = if med > 0.0 { best / med } else { f64::INFINITY };
    let confidence = best / total_weight.max(1.0);

    // Refine: weighted mean over the peak window plus a margin, which averages out noise.
    let lo = best_i.saturating_sub(6);
    let hi = (best_i + win + 6).min(bins);
    let weight_sum: f64 = hist[lo..hi].iter().sum();
    if weight_sum <= 0.0 {
        return None;
    }
    let center: f64 = (lo..hi)
        .map(|b| (-RANGE + (b as f64 + 0.5) * BIN) * hist[b])
        .sum::<f64>()
        / weight_sum;

    let min_conf = if vertical { 0.04 } else { 0.03 };
    if confidence >= min_conf && dominance >= 3.0 {
        Some((center, confidence))
    } else {
        None
    }
}

fn horizon_score(deg: f64) -> f64 {
    let d = deg.abs();
    if d <= 0.5 {
        5.0
    } else {
        clamp(5.0 - (d - 0.5) * 0.8, 0.0, 5.0)
    }
}

fn dhash(gray: &image::GrayImage) -> u64 {
    let small = imageops::resize(gray, 9, 8, imageops::FilterType::Triangle);
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            bits <<= 1;
            if small.get_pixel(x, y).0[0] > small.get_pixel(x + 1, y).0[0] {
                bits |= 1;
            }
        }
    }
    bits
}

pub fn analyze_image(img: &DynamicImage, exif: &ExifInfo) -> Metrics {
    let small = if img.width().max(img.height()) > ANALYSIS_EDGE {
        img.thumbnail(ANALYSIS_EDGE, ANALYSIS_EDGE)
    } else {
        img.clone()
    };
    let gray = small.to_luma8();
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    let hash = dhash(&gray);

    if w < GRID * 8 || h < GRID * 8 {
        return Metrics {
            sharpness: 2.5,
            shake: 2.5,
            exposure: exposure_score(gray.as_raw()),
            noise: 2.5,
            horizon: None,
            horizon_deg: None,
            sharp_region: 0.0,
            dhash: hash,
            edge_strength: 0.0,
            axis_ratio: 1.0,
        };
    }

    let px = gray.as_raw();
    let grads = sobel(px, w, h);
    let tiles = tiles(&grads);

    // Noise from the flattest tiles, where edges can't be mistaken for noise.
    let mut by_flatness: Vec<&Tile> = tiles.iter().collect();
    by_flatness.sort_by(|a, b| a.mean.partial_cmp(&b.mean).unwrap());
    let mut sigmas: Vec<f64> = by_flatness.iter().take(16).map(|t| tile_noise(px, w, t)).collect();
    let sigma = median(&mut sigmas);

    // Sharpness: edge strength relative to the contrast available in the same tile, taken
    // over the best-focused ~10% of the frame. A sharp subject with deliberately blurred
    // surroundings (or the reverse) still passes; dividing by contrast keeps a soft but
    // contrasty scene from looking "sharper" than a crisp low-contrast one.
    struct Scored<'a> {
        tile: &'a Tile,
        norm: f64,
    }
    let mut scored: Vec<Scored> = tiles
        .iter()
        .filter_map(|t| {
            let range = tile_range(px, w, t) as f64;
            if range < 25.0 {
                return None; // flat tile (sky, wall): says nothing about focus
            }
            // Sensor noise adds fake edge energy; remove the noise-driven part.
            let edge = (t.edge as f64 - 2.0 * sigma).max(0.0);
            Some(Scored { tile: t, norm: edge / range })
        })
        .collect();
    scored.sort_by(|a, b| b.norm.partial_cmp(&a.norm).unwrap());
    let top: Vec<&Scored> = scored.iter().take(6).collect();
    let normalized = if top.is_empty() {
        0.12 // featureless frame: neither clearly sharp nor blurred
    } else {
        top.iter().map(|s| s.norm).sum::<f64>() / top.len() as f64
    };
    let sharpness = sharpness_score(normalized);
    let sharp_region = if top.is_empty() {
        0.0
    } else {
        scored.iter().filter(|s| s.norm >= normalized * 0.6).count() as f64 / tiles.len() as f64
    };

    // Motion blur smears edges along one axis: a lopsided x/y edge energy in the sharpest
    // tiles. Camera EXIF (shutter vs focal length) adds a mild prior. Scenes with strong
    // natural direction (a horizon, tree trunks) can also look lopsided, so the penalty is
    // deliberately gentle.
    let sx: f32 = top.iter().map(|s| s.tile.abs_gx).sum();
    let sy: f32 = top.iter().map(|s| s.tile.abs_gy).sum();
    let ratio = (sx.max(sy) / (sx.min(sy) + 1.0)) as f64;
    let mut shake_pen = clamp((ratio - 2.5) / 3.5, 0.0, 1.0) * 2.5;
    if let (Some(t), Some(f)) = (exif.exposure_time, exif.focal35) {
        let r = t * f; // reciprocal rule: <= 1 is comfortable handheld
        if r > 1.0 {
            shake_pen += clamp(r.log2(), 0.0, 3.0) * 0.5;
        }
    }
    let shake = clamp(5.0 - shake_pen, 0.0, 5.0);

    // Horizon / verticals from the strongest 5% of edge pixels.
    let mut mags = grads.mag.clone();
    let idx = ((mags.len() as f32 * 0.95) as usize).min(mags.len() - 1);
    mags.select_nth_unstable_by(idx, |a, b| a.partial_cmp(b).unwrap());
    let strong_thr = mags[idx].max(1.0);
    let total_weight: f64 = grads.mag.iter().filter(|&&m| m >= strong_thr).map(|&m| m as f64).sum();
    let tilt = dominant_tilt(&grads, strong_thr, false, total_weight)
        .or_else(|| dominant_tilt(&grads, strong_thr, true, total_weight));
    let (horizon, horizon_deg) = match tilt {
        Some((deg, _)) => (Some(horizon_score(deg)), Some(deg)),
        None => (None, None),
    };

    Metrics {
        sharpness,
        shake,
        exposure: exposure_score(px),
        noise: noise_score(sigma),
        horizon,
        horizon_deg,
        sharp_region,
        dhash: hash,
        edge_strength: normalized * 100.0,
        axis_ratio: ratio,
    }
}

// ---------------------------------------------------------------------------
// EXIF
// ---------------------------------------------------------------------------

/// "2024:05:01 12:00:00.05+02:00" -> seconds since an arbitrary epoch (only differences matter).
fn parse_capture_time(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let frac = s
        .get(19..)
        .and_then(|rest| rest.strip_prefix('.'))
        .map(|rest| rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .filter(|f| !f.is_empty())
        .and_then(|f| format!("0.{}", f).parse::<f64>().ok())
        .unwrap_or(0.0);
    // Days from civil (Howard Hinnant's algorithm).
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + h * 3600 + mi * 60 + se) as f64 + frac)
}

fn read_exif_batch(app: &AppHandle, paths: &[String]) -> HashMap<String, ExifInfo> {
    let mut result = HashMap::new();
    let Ok(tool) = exiftool::exiftool_path(app) else {
        return result;
    };
    let base = [
        "-charset",
        "filename=utf8",
        "-j",
        "-SubSecDateTimeOriginal",
        "-ExposureTime#",
        "-FocalLengthIn35mmFormat#",
    ];
    for entry in exiftool::run_json(&tool, &base, paths) {
        let Some(src) = entry.get("SourceFile").and_then(Value::as_str) else {
            continue;
        };
        let info = ExifInfo {
            capture_time: entry
                .get("SubSecDateTimeOriginal")
                .and_then(Value::as_str)
                .and_then(parse_capture_time),
            exposure_time: entry.get("ExposureTime").and_then(Value::as_f64),
            focal35: entry.get("FocalLengthIn35mmFormat").and_then(Value::as_f64),
        };
        // exiftool reports forward-slash paths; map back to the caller's spelling.
        let key = paths
            .iter()
            .find(|p| p.replace('\\', "/") == src)
            .cloned()
            .unwrap_or_else(|| src.to_string());
        result.insert(key, info);
    }
    result
}

// ---------------------------------------------------------------------------
// Series grouping
// ---------------------------------------------------------------------------

/// Chains photos shot close together in time that also look alike (see `linked` below for
/// the time/similarity trade-off).
fn recluster(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("UPDATE analysis SET series_id = NULL", [])?;

    struct Row {
        path: String,
        time: f64,
        hash: u64,
    }
    let mut stmt = conn.prepare(
        "SELECT a.path, a.capture_time, a.dhash, p.mtime
         FROM analysis a JOIN photos p ON p.path = a.path
         WHERE p.rated = 0 AND a.version = ?1",
    )?;
    let mut rows: Vec<Row> = stmt
        .query_map([ANALYSIS_VERSION], |r| {
            let t: Option<f64> = r.get(1)?;
            let h: Option<i64> = r.get(2)?;
            let m: i64 = r.get(3)?;
            Ok(Row {
                path: r.get(0)?,
                time: t.unwrap_or(m as f64),
                hash: h.unwrap_or(0) as u64,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    rows.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap().then_with(|| a.path.cmp(&b.path)));

    let mut chain: Vec<usize> = Vec::new();
    let flush = |chain: &mut Vec<usize>, rows: &Vec<Row>| -> rusqlite::Result<()> {
        if chain.len() >= 2 {
            let mut hasher = Sha256::new();
            hasher.update(rows[chain[0]].path.as_bytes());
            let id = format!("{:x}", hasher.finalize())[..12].to_string();
            for &i in chain.iter() {
                conn.execute("UPDATE analysis SET series_id = ?1 WHERE path = ?2", params![id, rows[i].path])?;
            }
        }
        chain.clear();
        Ok(())
    };
    for i in 0..rows.len() {
        if let Some(&prev) = chain.last() {
            let dt = rows[i].time - rows[prev].time;
            let dist = (rows[i].hash ^ rows[prev].hash).count_ones();
            // The longer the gap, the more alike two frames must look. A pair that is
            // borderline on both counts (e.g. 4 s apart AND 15 bits different) is not a series.
            let linked = (dt <= 2.0 && dist <= 16) || (dt <= 4.0 && dist <= 12) || (dt <= 15.0 && dist <= 8);
            if !linked {
                flush(&mut chain, &rows)?;
            }
        }
        chain.push(i);
    }
    flush(&mut chain, &rows)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
}

#[derive(Serialize, Clone)]
struct AnalysisDone {
    analyzed: usize,
    failed: usize,
}

/// Analyses every *unrated* photo in the folder's catalogue that has no up-to-date result.
/// Runs in the background; emits `analysis-progress`, `analysis-record`, `analysis-complete`.
#[tauri::command]
pub fn analyze_folder(app: AppHandle, window: tauri::Window, folder: String) {
    std::thread::spawn(move || {
        if let Err(e) = run_analysis(&app, &window, &folder) {
            let _ = window.emit("analysis-error", e);
        }
    });
}

fn run_analysis(app: &AppHandle, window: &tauri::Window, folder: &str) -> Result<(), String> {
    let conn = crate::catalog::open_db(app, folder)?;
    let todo: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT p.path FROM photos p LEFT JOIN analysis a ON a.path = p.path
                 WHERE p.rated = 0
                   AND (a.path IS NULL OR a.version <> ?1 OR a.analyzed_at < p.mtime)",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([ANALYSIS_VERSION], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let total = todo.len();
    let _ = window.emit("analysis-progress", Progress { done: 0, total });

    let exif = Arc::new(read_exif_batch(app, &todo));
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cores.saturating_sub(1).max(1))
        .build()
        .map_err(|e| e.to_string())?;

    let (tx, rx) = mpsc::channel::<(String, Option<Metrics>)>();
    let worker_paths = todo.clone();
    let worker_exif = Arc::clone(&exif);
    let worker = std::thread::spawn(move || {
        pool.install(|| {
            worker_paths.par_iter().for_each_with(tx, |tx, path| {
                let info = worker_exif.get(path).cloned().unwrap_or_default();
                let metrics = image::open(path).ok().map(|img| analyze_image(&img, &info));
                let _ = tx.send((path.clone(), metrics));
            });
        });
    });

    let (mut done, mut analyzed, mut failed) = (0usize, 0usize, 0usize);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    for (path, metrics) in rx {
        done += 1;
        match metrics {
            Some(m) => {
                let info = exif.get(&path).cloned().unwrap_or_default();
                conn.execute(
                    "INSERT OR REPLACE INTO analysis
                     (path, version, analyzed_at, sharpness, shake, exposure, noise, horizon,
                      horizon_deg, sharp_region, capture_time, dhash, series_id, ai_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,NULL)",
                    params![
                        path,
                        ANALYSIS_VERSION,
                        now,
                        m.sharpness,
                        m.shake,
                        m.exposure,
                        m.noise,
                        m.horizon,
                        m.horizon_deg,
                        m.sharp_region,
                        info.capture_time,
                        m.dhash as i64
                    ],
                )
                .map_err(|e| e.to_string())?;
                analyzed += 1;
                let _ = window.emit(
                    "analysis-record",
                    AnalysisRecord {
                        path,
                        sharpness: m.sharpness,
                        shake: m.shake,
                        exposure: m.exposure,
                        noise: m.noise,
                        horizon: m.horizon,
                        horizon_deg: m.horizon_deg,
                        series_id: None,
                        ai: None,
                    },
                );
            }
            None => failed += 1,
        }
        let _ = window.emit("analysis-progress", Progress { done, total });
    }
    let _ = worker.join();

    recluster(&conn).map_err(|e| e.to_string())?;
    let _ = window.emit("analysis-complete", AnalysisDone { analyzed, failed });
    Ok(())
}

#[tauri::command]
pub fn load_analysis(app: AppHandle, folder: String) -> Result<Vec<AnalysisRecord>, String> {
    let conn = crate::catalog::open_db(&app, &folder)?;
    let mut stmt = conn
        .prepare(
            "SELECT a.path, a.sharpness, a.shake, a.exposure, a.noise, a.horizon, a.horizon_deg, a.series_id, a.ai_json
             FROM analysis a JOIN photos p ON p.path = a.path
             WHERE a.version = ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([ANALYSIS_VERSION], |r| {
            Ok(AnalysisRecord {
                path: r.get(0)?,
                sharpness: r.get(1)?,
                shake: r.get(2)?,
                exposure: r.get(3)?,
                noise: r.get(4)?,
                horizon: r.get(5)?,
                horizon_deg: r.get(6)?,
                series_id: r.get(7)?,
                ai: r
                    .get::<_, Option<String>>(8)?
                    .and_then(|json| serde_json::from_str(&json).ok()),
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[derive(Deserialize)]
pub struct AutoRating {
    pub path: String,
    pub rating: i32,
}

#[derive(Serialize, Clone)]
struct ApplyDone {
    written: usize,
    skipped: usize,
    failed: usize,
}

/// Writes star ratings computed from the auto score - but only to photos that are *still*
/// unrated on disk right now, so a rating added elsewhere in the meantime is never overwritten.
#[tauri::command]
pub fn apply_auto_ratings(app: AppHandle, window: tauri::Window, folder: String, items: Vec<AutoRating>) {
    std::thread::spawn(move || {
        let total = items.len();
        let _ = window.emit("apply-progress", Progress { done: 0, total });
        let paths: Vec<PathBuf> = items.iter().map(|i| PathBuf::from(&i.path)).collect();
        let live = exiftool::read_ratings_batch(&app, &paths);
        let conn = crate::catalog::open_db(&app, &folder).ok();

        let (mut written, mut skipped, mut failed) = (0, 0, 0);
        for (n, item) in items.iter().enumerate() {
            if live.contains_key(&PathBuf::from(&item.path).to_string_lossy().to_string()) {
                skipped += 1;
            } else {
                let raw: Option<String> = conn.as_ref().and_then(|c| {
                    c.query_row("SELECT raw_path FROM photos WHERE path = ?1", [&item.path], |r| r.get(0))
                        .ok()
                        .flatten()
                });
                let rating = item.rating.clamp(0, 5);
                match exiftool::write_rating_files(&app, &item.path, raw.as_deref(), rating) {
                    Ok(()) => {
                        record_rating(&app, &folder, &item.path, rating);
                        written += 1;
                    }
                    Err(_) => failed += 1,
                }
            }
            let _ = window.emit("apply-progress", Progress { done: n + 1, total });
        }
        let _ = window.emit("apply-complete", ApplyDone { written, skipped, failed });
    });
}

// ---------------------------------------------------------------------------
// Calibration helpers (run with: NAJSU_CALIB_DIR=<folder of jpgs> cargo test calibrate -- --ignored --nocapture)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(m: &Metrics) -> String {
        format!(
            "norm {:5.1} ratio {:4.1} sharp {:.2}  shake {:.2}  expo {:.2}  noise {:.2}  horizon {}",
            m.edge_strength,
            m.axis_ratio,
            m.sharpness,
            m.shake,
            m.exposure,
            m.noise,
            match (m.horizon, m.horizon_deg) {
                (Some(s), Some(d)) => format!("{:.2} ({:+.2} deg)", s, d),
                _ => "n/a".into(),
            }
        )
    }

    fn add_noise(img: &DynamicImage, amp: f32) -> DynamicImage {
        let mut rgb = img.to_rgb8();
        let mut s: u32 = 12345;
        for p in rgb.pixels_mut() {
            for c in p.0.iter_mut() {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                let r = ((s >> 16) & 0xFF) as f32 / 255.0 - 0.5;
                *c = (*c as f32 + r * amp * 3.46).clamp(0.0, 255.0) as u8;
            }
        }
        DynamicImage::ImageRgb8(rgb)
    }

    /// Horizontal box blur of `len` pixels: what sideways camera shake does to edges.
    fn motion_blur(img: &DynamicImage, len: u32) -> DynamicImage {
        let src = img.to_rgb8();
        let (w, h) = (src.width(), src.height());
        let mut out = src.clone();
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0u32; 3];
                for k in 0..len {
                    let sx = (x + k).saturating_sub(len / 2).min(w - 1);
                    let p = src.get_pixel(sx, y).0;
                    for c in 0..3 {
                        acc[c] += p[c] as u32;
                    }
                }
                out.put_pixel(x, y, image::Rgb([(acc[0] / len) as u8, (acc[1] / len) as u8, (acc[2] / len) as u8]));
            }
        }
        DynamicImage::ImageRgb8(out)
    }

    /// Bilinear rotation about the centre (anti-aliased, like a real tilted horizon).
    fn rotate(img: &DynamicImage, deg: f32) -> DynamicImage {
        let src = img.to_rgb8();
        let (w, h) = (src.width() as i32, src.height() as i32);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let (s, c) = deg.to_radians().sin_cos();
        let mut out = src.clone();
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                let fx = (c * dx + s * dy + cx).clamp(0.0, (w - 1) as f32 - 0.001);
                let fy = (-s * dx + c * dy + cy).clamp(0.0, (h - 1) as f32 - 0.001);
                let (x0, y0) = (fx as u32, fy as u32);
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let mut px = [0u8; 3];
                for ch in 0..3 {
                    let p = |xx: u32, yy: u32| src.get_pixel(xx, yy).0[ch] as f32;
                    let top = p(x0, y0) * (1.0 - tx) + p(x0 + 1, y0) * tx;
                    let bot = p(x0, y0 + 1) * (1.0 - tx) + p(x0 + 1, y0 + 1) * tx;
                    px[ch] = (top * (1.0 - ty) + bot * ty).round() as u8;
                }
                out.put_pixel(x as u32, y as u32, image::Rgb(px));
            }
        }
        DynamicImage::ImageRgb8(out)
    }

    /// Sky over textured ground with a hard horizon, rotated by known angles.
    #[test]
    fn capture_time_parsing_handles_subseconds_and_day_rollover() {
        let a = parse_capture_time("2024:05:01 12:00:00").unwrap();
        let b = parse_capture_time("2024:05:01 12:00:00.05+02:00").unwrap();
        assert!((b - a - 0.05).abs() < 1e-6);
        let before_midnight = parse_capture_time("2024:02:28 23:59:59").unwrap();
        let after = parse_capture_time("2024:02:29 00:00:01").unwrap(); // leap day
        assert!((after - before_midnight - 2.0).abs() < 1e-6);
        assert!(parse_capture_time("garbage").is_none());
    }

    #[test]
    fn series_groups_bursts_and_ignores_rated_photos() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE photos (path TEXT PRIMARY KEY, mtime INTEGER, rated INTEGER);",
        )
        .unwrap();
        init_schema(&conn).unwrap();

        // (path, capture time, perceptual hash, rated)
        let rows: [(&str, f64, u64, i64); 8] = [
            ("a.jpg", 100.0, 0, 0),
            ("b.jpg", 101.0, 0b11, 0),             // burst: 1 s later, near-identical
            ("c.jpg", 102.0, u64::MAX >> 4, 0),    // 1 s later but a different picture
            ("d.jpg", 500.0, 0, 0),                // looks like a.jpg but minutes later
            ("e.jpg", 100.5, 0, 1),                // would join the burst, but is rated
            ("f.jpg", 101.5, 0b1, 0),              // also joins the burst
            // Borderline on both counts: 4 s apart and 15 bits different (real case DSC00711/712).
            ("g.jpg", 900.0, 0, 0),
            ("h.jpg", 904.0, 0x7FFF, 0),
        ];
        for (path, t, hash, rated) in rows {
            conn.execute("INSERT INTO photos VALUES (?1, 0, ?2)", params![path, rated]).unwrap();
            conn.execute(
                "INSERT INTO analysis (path, version, analyzed_at, sharpness, shake, exposure, noise, capture_time, dhash)
                 VALUES (?1, ?2, 0, 3, 3, 3, 3, ?3, ?4)",
                params![path, ANALYSIS_VERSION, t, hash as i64],
            )
            .unwrap();
        }
        recluster(&conn).unwrap();

        let series = |p: &str| -> Option<String> {
            conn.query_row("SELECT series_id FROM analysis WHERE path = ?1", [p], |r| r.get(0)).unwrap()
        };
        assert!(series("a.jpg").is_some());
        assert_eq!(series("a.jpg"), series("b.jpg"));
        assert_eq!(series("a.jpg"), series("f.jpg"));
        assert_eq!(series("c.jpg"), None, "different picture must not join");
        assert_eq!(series("d.jpg"), None, "same look but minutes apart is not a series");
        assert_eq!(series("e.jpg"), None, "rated photos are left alone");
        assert_eq!(series("g.jpg"), None, "borderline time AND similarity must not link");
        assert_eq!(series("h.jpg"), None);
    }

    #[test]
    fn horizon_is_measured() {
        let (w, h) = (1200u32, 800u32);
        let mut img = image::RgbImage::new(w, h);
        let mut s: u32 = 7;
        for y in 0..h {
            for x in 0..w {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                let n = ((s >> 24) as i32 % 24) - 12;
                let v = if y < h / 2 {
                    (170 + (y / 8) as i32).clamp(0, 255)
                } else {
                    (70 + n).clamp(0, 255)
                };
                img.put_pixel(x, y, image::Rgb([v as u8, v as u8, (v + 20).min(255) as u8]));
            }
        }
        let base = DynamicImage::ImageRgb8(img);
        for deg in [0.0f32, 1.0, 3.0, 6.0] {
            let m = analyze_image(&rotate(&base, deg), &ExifInfo::default());
            println!("tilt {deg}: {}", fmt(&m));
            let measured = m.horizon_deg.expect("a hard horizon must be detected");
            // The Sobel-based estimate reads steep tilts ~10-15% low; fine for a "is it level?" score.
            let tolerance = 0.35 + 0.15 * deg as f64;
            assert!(
                (measured.abs() - deg as f64).abs() <= tolerance,
                "tilted {deg} deg but measured {measured}"
            );
        }
    }

    #[test]
    #[ignore]
    fn calibrate() {
        let dir = std::env::var("NAJSU_CALIB_DIR").expect("set NAJSU_CALIB_DIR");
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        for path in files.into_iter().take(12) {
            let Ok(img) = image::open(&path) else { continue };
            // Degrade at analysis resolution, otherwise the downscale hides the effect.
            let img = img.thumbnail(ANALYSIS_EDGE, ANALYSIS_EDGE);
            let exif = ExifInfo::default();
            println!("\n{}", path.file_name().unwrap().to_string_lossy());
            println!("  original      {}", fmt(&analyze_image(&img, &exif)));
            for sigma in [1.0f32, 2.0, 4.0] {
                println!("  blur {:<8}  {}", sigma, fmt(&analyze_image(&img.blur(sigma), &exif)));
            }
            for len in [9u32, 21] {
                println!("  motion {:<6}  {}", len, fmt(&analyze_image(&motion_blur(&img, len), &exif)));
            }
            for amp in [4.0f32, 12.0] {
                println!("  noise {:<7}  {}", amp, fmt(&analyze_image(&add_noise(&img, amp), &exif)));
            }
            for deg in [1.0f32, 3.0, 6.0] {
                println!("  tilt {:<8}  {}", deg, fmt(&analyze_image(&rotate(&img, deg), &exif)));
            }
        }
    }
}

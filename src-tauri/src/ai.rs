//! Local AI review of photos through an Ollama server running on this computer.
//!
//! Privacy is enforced in code, not just by policy:
//!  * the HTTP client is built without TLS, so it cannot speak to any https:// service;
//!  * `validate_endpoint` only accepts http://localhost, 127.0.0.1 and [::1];
//!  * redirects are disabled, so a local server cannot bounce a request elsewhere.
//! Photos are downscaled in memory and sent only to that local address.

use base64::Engine;
use image::{DynamicImage, ImageDecoder, ImageReader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

static CANCEL: AtomicBool = AtomicBool::new(false);

/// Categories the local pixel analysis cannot judge. Scores are 0-5.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AiResult {
    pub model: String,
    pub composition: f64,
    pub subject: f64,
    /// None when nobody is visible in the photo.
    pub people: Option<f64>,
    pub impact: f64,
    pub summary: String,
}

const PROMPT: &str = "You are assisting a photographer who is culling a shoot. Judge ONLY the \
aesthetic and content qualities below. Do NOT judge technical sharpness, noise, exposure or \
horizon tilt: those are measured separately. Photographers often deliberately use shallow depth \
of field, focusing on the background or leaving the foreground subject soft, so treat deliberate \
blur as an artistic choice, never as a flaw.\n\
Score each from 0 to 5 (decimals allowed):\n\
- composition: framing, balance, subject placement, leading lines, use of space, awkward cropping \
of the subject or the edges, tangents.\n\
- subject: how clear the main subject is, how well it separates from the background, and whether \
distracting elements compete with it.\n\
- people: ONLY if people are visible: eyes open, natural expression, flattering pose, no awkward \
body position or mid-blink. If nobody is visible set people_present to false and people to 0.\n\
- impact: overall, would a photographer keep this as a strong image (moment, mood, light, interest).\n\
Scale: 5 exceptional, 4 good, 3 decent but unremarkable, 2 weak, 1 poor, 0 unusable. Most ordinary \
photos belong between 2 and 4; use the full range.\n\
Reply with JSON only. In summary write one short sentence (max 25 words) naming the main strength \
or flaw.";

fn response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "composition": {"type": "number"},
            "subject": {"type": "number"},
            "people_present": {"type": "boolean"},
            "people": {"type": "number"},
            "impact": {"type": "number"},
            "summary": {"type": "string"}
        },
        "required": ["composition", "subject", "people_present", "people", "impact", "summary"]
    })
}

/// Accepts only plain-http addresses on this machine; returns the endpoint without a trailing slash.
pub fn validate_endpoint(endpoint: &str) -> Result<String, String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    let rest = trimmed.strip_prefix("http://").ok_or_else(|| {
        "Only plain http://localhost addresses are allowed: photos never leave this computer.".to_string()
    })?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') {
        return Err("The address must not contain credentials.".into());
    }
    let host = if authority.starts_with('[') {
        authority.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    match host.to_ascii_lowercase().as_str() {
        "localhost" | "127.0.0.1" | "[::1]" => Ok(trimmed.to_string()),
        other => Err(format!(
            "'{other}' is not a local address. Photos are only ever sent to this computer (localhost / 127.0.0.1)."
        )),
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(600)) // a CPU-only vision model can take a long time
        .build()
}

fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
                .unwrap_or(body);
            format!("Ollama answered {code}: {detail}")
        }
        other => format!("Could not reach Ollama: {other}"),
    }
}

/// Loads an image upright (respecting the EXIF orientation) and downsizes it to `edge` px.
fn prepare_image(path: &str, edge: u32) -> Result<String, String> {
    let mut decoder = ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let small = if img.width().max(img.height()) > edge {
        img.thumbnail(edge, edge)
    } else {
        img
    };

    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85)
        .encode_image(&small.to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn score(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .map(|s| s.clamp(0.0, 5.0))
        .ok_or_else(|| format!("The model's answer is missing '{key}'."))
}

/// Pulls the JSON object out of a model reply, tolerating code fences or stray prose.
pub fn parse_reply(content: &str, model: &str) -> Result<AiResult, String> {
    let start = content.find('{').ok_or("The model did not return JSON.")?;
    let end = content.rfind('}').ok_or("The model did not return JSON.")?;
    let v: Value = serde_json::from_str(&content[start..=end])
        .map_err(|e| format!("The model's JSON could not be read: {e}"))?;

    let present = v.get("people_present").and_then(Value::as_bool).unwrap_or(false);
    Ok(AiResult {
        model: model.to_string(),
        composition: score(&v, "composition")?,
        subject: score(&v, "subject")?,
        people: if present { Some(score(&v, "people")?) } else { None },
        impact: score(&v, "impact")?,
        summary: v
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .chars()
            .take(240)
            .collect(),
    })
}

/// Sends one chat request to the local Ollama and returns the model's text answer.
fn chat(endpoint: &str, model: &str, prompt: &str, images: &[String], schema: Value) -> Result<String, String> {
    let body = json!({
        "model": model,
        "stream": false,
        "format": schema,
        "options": {"temperature": 0},
        "messages": [{"role": "user", "content": prompt, "images": images}]
    });
    let reply: Value = agent()
        .post(&format!("{endpoint}/api/chat"))
        .send_json(body)
        .map_err(describe)?
        .into_json()
        .map_err(|e| format!("Unreadable answer from Ollama: {e}"))?;
    reply
        .pointer("/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Ollama's answer had no message content.".to_string())
}

pub fn review_image(endpoint: &str, model: &str, image_b64: &str) -> Result<AiResult, String> {
    let content = chat(endpoint, model, PROMPT, &[image_b64.to_string()], response_schema())?;
    parse_reply(&content, model)
}

// ---------------------------------------------------------------------------
// Series comparison
// ---------------------------------------------------------------------------

fn series_prompt(n: usize) -> String {
    format!(
        "You are assisting a photographer choosing the single best frame from a burst. The {n} images are consecutive frames of the same scene, in the order shot, numbered 1 to {n}. Rank them from best to worst as a keeper.
Judge: the decisive moment, expression and eyes (open, natural), pose and gesture, composition, and how clearly the subject stands out. Do NOT judge technical sharpness, noise or exposure: those are measured separately. Deliberate background focus or a soft foreground subject is an artistic choice, not a flaw. Do not favour a frame because of its position in the list.
Reply with JSON only: \"ranking\" is a list of all {n} numbers, best first, each number exactly once; \"reason\" is one short sentence (max 25 words) saying why the first one wins."
    )
}

fn ranking_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "ranking": {"type": "array", "items": {"type": "integer"}},
            "reason": {"type": "string"}
        },
        "required": ["ranking", "reason"]
    })
}

/// Reads a ranking reply: returns 0-based indices, best first. The list must be a permutation of
/// 1..=n; a model that repeats or skips a frame has not really ranked them, so it's an error.
pub fn parse_ranking(content: &str, n: usize) -> Result<(Vec<usize>, String), String> {
    let start = content.find('{').ok_or("The model did not return JSON.")?;
    let end = content.rfind('}').ok_or("The model did not return JSON.")?;
    let v: Value = serde_json::from_str(&content[start..=end])
        .map_err(|e| format!("The model's JSON could not be read: {e}"))?;
    let list = v.get("ranking").and_then(Value::as_array).ok_or("The answer has no 'ranking'.")?;

    let mut seen = vec![false; n];
    let mut order = Vec::with_capacity(n);
    for item in list {
        let k = item.as_u64().ok_or("The ranking contains a non-number.")? as usize;
        if k < 1 || k > n || seen[k - 1] {
            return Err(format!("The ranking {list:?} is not a permutation of 1..{n}."));
        }
        seen[k - 1] = true;
        order.push(k - 1);
    }
    if order.len() != n {
        return Err(format!("The ranking has {} entries, expected {n}.", order.len()));
    }
    let reason: String = v.get("reason").and_then(Value::as_str).unwrap_or("").trim().chars().take(240).collect();
    // A small model can name one image in its explanation and rank another first. A comment that
    // contradicts the pick would only mislead, so it is dropped.
    let reason = if reason_contradicts_pick(&reason, order[0]) { String::new() } else { reason };
    Ok((order, reason))
}

/// True if the first "image N" / "photo N" / "frame N" in the text names a frame other than `best`.
fn reason_contradicts_pick(reason: &str, best: usize) -> bool {
    let lower = reason.to_lowercase();
    let first_mention = ["image ", "photo ", "frame ", "picture "]
        .iter()
        .filter_map(|kw| {
            let at = lower.find(kw)?;
            let digits: String = lower[at + kw.len()..].chars().take_while(|c| c.is_ascii_digit()).collect();
            Some((at, digits.parse::<usize>().ok()?))
        })
        .min_by_key(|(at, _)| *at);
    matches!(first_mention, Some((_, n)) if n != best + 1)
}

/// One ranking pass over `images` (which may already be in a shuffled order).
fn rank_pass(endpoint: &str, model: &str, images: &[String]) -> Result<(Vec<usize>, String), String> {
    let content = chat(endpoint, model, &series_prompt(images.len()), images, ranking_schema())?;
    parse_ranking(&content, images.len())
}

/// Merges two passes (frames shown in order, then reversed) into a final order plus per-frame
/// points. Small models lean towards the first or last image, so a frame that wins in only one
/// of the two orders earns the same points as its rival and the caller can break the tie with
/// something less biased.
pub fn combine_passes(n: usize, forward: &[usize], reversed: &[usize]) -> (Vec<usize>, Vec<f64>) {
    let mut points = vec![0.0; n];
    for (pos, &idx) in forward.iter().enumerate() {
        points[idx] += (n - pos) as f64;
    }
    // In the reversed pass the model saw frame k as image n-1-k, so map its answer back.
    for (pos, &idx) in reversed.iter().enumerate() {
        points[n - 1 - idx] += (n - pos) as f64;
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| points[b].partial_cmp(&points[a]).unwrap().then(a.cmp(&b)));
    (order, points)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct OllamaModel {
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct OllamaStatus {
    pub reachable: bool,
    pub models: Vec<OllamaModel>,
    pub error: Option<String>,
}

/// Is Ollama running locally, and which models are installed?
#[tauri::command]
pub fn ai_check(endpoint: String) -> OllamaStatus {
    let result = validate_endpoint(&endpoint).and_then(|ep| {
        agent()
            .get(&format!("{ep}/api/tags"))
            .call()
            .map_err(describe)?
            .into_json::<Value>()
            .map_err(|e| e.to_string())
    });
    match result {
        Ok(v) => OllamaStatus {
            reachable: true,
            models: v
                .get("models")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|m| {
                            Some(OllamaModel {
                                name: m.get("name")?.as_str()?.to_string(),
                                size_bytes: m.get("size").and_then(Value::as_u64).unwrap_or(0),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            error: None,
        },
        Err(e) => OllamaStatus { reachable: false, models: vec![], error: Some(e) },
    }
}

#[tauri::command]
pub fn ai_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

#[derive(Serialize, Clone)]
struct AiProgress {
    done: usize,
    total: usize,
    /// Seconds spent on the photo that just finished (drives the ETA in the UI).
    last_secs: f64,
}

#[derive(Serialize, Clone)]
struct AiRecord {
    path: String,
    ai: AiResult,
}

#[derive(Serialize, Clone)]
struct AiDone {
    reviewed: usize,
    failed: usize,
    cancelled: bool,
}

/// Reviews the given photos one at a time (a local model is the bottleneck, so there is
/// nothing to gain from parallel requests). Emits `ai-progress`, `ai-record`, `ai-complete`.
#[tauri::command]
pub fn ai_review(
    app: AppHandle,
    window: tauri::Window,
    folder: String,
    paths: Vec<String>,
    endpoint: String,
    model: String,
    image_edge: u32,
) -> Result<(), String> {
    let endpoint = validate_endpoint(&endpoint)?;
    CANCEL.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        let total = paths.len();
        let (mut done, mut reviewed, mut failed, mut consecutive) = (0usize, 0usize, 0usize, 0usize);
        let mut last_error = String::new();
        let _ = window.emit("ai-progress", AiProgress { done: 0, total, last_secs: 0.0 });

        for path in &paths {
            if CANCEL.load(Ordering::SeqCst) {
                break;
            }
            let started = Instant::now();
            let outcome = prepare_image(path, image_edge.clamp(256, 2048))
                .and_then(|b64| review_image(&endpoint, &model, &b64));
            match outcome {
                Ok(ai) => {
                    consecutive = 0;
                    reviewed += 1;
                    if let Ok(conn) = crate::catalog::open_db(&app, &folder) {
                        let _ = conn.execute(
                            "UPDATE analysis SET ai_json = ?1 WHERE path = ?2",
                            rusqlite::params![serde_json::to_string(&ai).unwrap_or_default(), path],
                        );
                    }
                    let _ = window.emit("ai-record", AiRecord { path: path.clone(), ai });
                }
                Err(e) => {
                    failed += 1;
                    consecutive += 1;
                    last_error = e;
                }
            }
            done += 1;
            let _ = window.emit(
                "ai-progress",
                AiProgress { done, total, last_secs: started.elapsed().as_secs_f64() },
            );
            // If the server is down or the model is missing, every photo would fail the same way.
            if consecutive >= 3 {
                let _ = window.emit("ai-error", last_error.clone());
                break;
            }
        }
        let cancelled = CANCEL.load(Ordering::SeqCst);
        let _ = window.emit("ai-complete", AiDone { reviewed, failed, cancelled });
    });
    Ok(())
}

#[derive(Deserialize)]
pub struct SeriesJob {
    pub id: String,
    /// Frames in the order they were shot.
    pub paths: Vec<String>,
}

/// The AI's verdict on one series: frame paths best first, and their points.
#[derive(Serialize, Clone)]
pub struct SeriesRanking {
    pub series_id: String,
    pub ranking: Vec<String>,
    pub points: Vec<f64>,
    pub reason: String,
    pub model: String,
    pub passes: u8,
}

#[derive(Serialize, Clone)]
struct SeriesDone {
    ranked: usize,
    failed: usize,
    cancelled: bool,
}

fn rank_series(
    endpoint: &str,
    model: &str,
    edge: u32,
    job: &SeriesJob,
    double_check: bool,
) -> Result<SeriesRanking, String> {
    let images: Vec<String> = job
        .paths
        .iter()
        .map(|p| prepare_image(p, edge))
        .collect::<Result<_, _>>()?;
    let n = images.len();

    let (forward, reason) = rank_pass(endpoint, model, &images)?;
    let (order, points, passes) = if double_check {
        let reversed_images: Vec<String> = images.iter().rev().cloned().collect();
        let (reversed, _) = rank_pass(endpoint, model, &reversed_images)?;
        let (order, points) = combine_passes(n, &forward, &reversed);
        (order, points, 2u8)
    } else {
        let mut points = vec![0.0; n];
        for (pos, &idx) in forward.iter().enumerate() {
            points[idx] = (n - pos) as f64;
        }
        (forward, points, 1u8)
    };

    Ok(SeriesRanking {
        series_id: job.id.clone(),
        ranking: order.iter().map(|&i| job.paths[i].clone()).collect(),
        points: order.iter().map(|&i| points[i]).collect(),
        reason,
        model: model.to_string(),
        passes,
    })
}

/// Compares the frames of each series side by side and stores which one wins.
/// Emits `ai-progress` (one step per series), `ai-series-record` and `ai-series-complete`.
#[tauri::command]
pub fn ai_rank_series(
    app: AppHandle,
    window: tauri::Window,
    folder: String,
    jobs: Vec<SeriesJob>,
    endpoint: String,
    model: String,
    image_edge: u32,
    double_check: bool,
) -> Result<(), String> {
    let endpoint = validate_endpoint(&endpoint)?;
    CANCEL.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        let total = jobs.len();
        let (mut done, mut ranked, mut failed, mut consecutive) = (0usize, 0usize, 0usize, 0usize);
        let mut last_error = String::new();
        let _ = window.emit("ai-progress", AiProgress { done: 0, total, last_secs: 0.0 });

        for job in &jobs {
            if CANCEL.load(Ordering::SeqCst) {
                break;
            }
            let started = Instant::now();
            match rank_series(&endpoint, &model, image_edge.clamp(256, 2048), job, double_check) {
                Ok(result) => {
                    consecutive = 0;
                    ranked += 1;
                    if let Ok(conn) = crate::catalog::open_db(&app, &folder) {
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO series_ai (series_id, members, points, reason, model, passes, created_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%s','now'))",
                            rusqlite::params![
                                result.series_id,
                                serde_json::to_string(&result.ranking).unwrap_or_default(),
                                serde_json::to_string(&result.points).unwrap_or_default(),
                                result.reason,
                                result.model,
                                result.passes
                            ],
                        );
                    }
                    let _ = window.emit("ai-series-record", result);
                }
                Err(e) => {
                    failed += 1;
                    consecutive += 1;
                    last_error = e;
                }
            }
            done += 1;
            let _ = window.emit(
                "ai-progress",
                AiProgress { done, total, last_secs: started.elapsed().as_secs_f64() },
            );
            if consecutive >= 3 {
                let _ = window.emit("ai-error", last_error.clone());
                break;
            }
        }
        let cancelled = CANCEL.load(Ordering::SeqCst);
        let _ = window.emit("ai-series-complete", SeriesDone { ranked, failed, cancelled });
    });
    Ok(())
}

#[tauri::command]
pub fn load_series_ai(app: AppHandle, folder: String) -> Result<Vec<SeriesRanking>, String> {
    let conn = crate::catalog::open_db(&app, &folder)?;
    let mut stmt = conn
        .prepare("SELECT series_id, members, points, reason, model, passes FROM series_ai")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let members: String = r.get(1)?;
            let points: String = r.get(2)?;
            Ok(SeriesRanking {
                series_id: r.get(0)?,
                ranking: serde_json::from_str(&members).unwrap_or_default(),
                points: serde_json::from_str(&points).unwrap_or_default(),
                reason: r.get(3)?,
                model: r.get(4)?,
                passes: r.get::<_, i64>(5)? as u8,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    const GOOD: &str = r#"{"composition":3.5,"subject":4,"people_present":true,"people":2.5,"impact":3,"summary":"Nice light, but the pose is awkward."}"#;

    /// A throwaway stand-in for Ollama: answers one request, reports what it received.
    fn mock_ollama(status: u16, body: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            let (mut header_end, mut content_len) = (None, 0usize);
            loop {
                let n = stream.read(&mut buf).unwrap();
                data.extend_from_slice(&buf[..n]);
                if header_end.is_none() {
                    if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = Some(i + 4);
                        let head = String::from_utf8_lossy(&data[..i]).to_ascii_lowercase();
                        content_len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                            .unwrap_or(0);
                    }
                }
                if let Some(h) = header_end {
                    if data.len() >= h + content_len || n == 0 {
                        break;
                    }
                }
            }
            let h = header_end.unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&data[h..]).to_string());
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (format!("http://127.0.0.1:{}", addr.port()), rx)
    }

    fn wrap(content: &str) -> String {
        json!({"message": {"role": "assistant", "content": content}}).to_string()
    }

    /// Real round trip against a locally running Ollama (opt-in):
    /// NAJSU_LIVE_MODEL=gemma3:4b NAJSU_LIVE_IMAGES="a.jpg;b.jpg" cargo test live_review -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_review() {
        let model = std::env::var("NAJSU_LIVE_MODEL").expect("set NAJSU_LIVE_MODEL");
        let images = std::env::var("NAJSU_LIVE_IMAGES").expect("set NAJSU_LIVE_IMAGES");
        let edge: u32 = std::env::var("NAJSU_LIVE_EDGE").ok().and_then(|v| v.parse().ok()).unwrap_or(768);
        for path in images.split(';').filter(|p| !p.is_empty()) {
            let started = Instant::now();
            let b64 = prepare_image(path, edge).expect("image could not be prepared");
            let prep = started.elapsed();
            let result = review_image("http://localhost:11434", &model, &b64);
            println!(
                "{path}
  prep {:.1}s, model {:.1}s ({} KB sent)
  {:?}",
                prep.as_secs_f64(),
                (started.elapsed() - prep).as_secs_f64(),
                b64.len() / 1024,
                result
            );
        }
    }

    /// Real series comparison against a locally running Ollama (opt-in):
    /// NAJSU_LIVE_MODEL=gemma3:4b NAJSU_LIVE_SERIES="a.jpg|b.jpg|c.jpg" cargo test live_rank -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_rank() {
        let model = std::env::var("NAJSU_LIVE_MODEL").expect("set NAJSU_LIVE_MODEL");
        let series = std::env::var("NAJSU_LIVE_SERIES").expect("set NAJSU_LIVE_SERIES");
        let paths: Vec<String> = series.split('|').filter(|p| !p.is_empty()).map(str::to_string).collect();
        let job = SeriesJob { id: "live".into(), paths };
        for double_check in [false, true] {
            let started = Instant::now();
            let result = rank_series("http://localhost:11434", &model, 768, &job, double_check);
            println!("double_check={double_check}: {:.0}s", started.elapsed().as_secs_f64());
            match result {
                Ok(r) => {
                    for (p, pts) in r.ranking.iter().zip(&r.points) {
                        println!("  {pts:>4}  {}", p.rsplit(['\\', '/']).next().unwrap_or(p));
                    }
                    println!("  reason: {}", r.reason);
                }
                Err(e) => println!("  ERROR: {e}"),
            }
        }
    }

    #[test]
    fn ranking_must_be_a_real_permutation() {
        let ok = parse_ranking(r#"{"ranking":[2,3,1],"reason":"Eyes open."}"#, 3).unwrap();
        assert_eq!(ok.0, vec![1, 2, 0]);
        assert_eq!(ok.1, "Eyes open.");
        assert!(parse_ranking(r#"{"ranking":[1,1,2],"reason":""}"#, 3).is_err(), "repeats");
        assert!(parse_ranking(r#"{"ranking":[1,2],"reason":""}"#, 3).is_err(), "skips a frame");
        assert!(parse_ranking(r#"{"ranking":[1,2,4],"reason":""}"#, 3).is_err(), "out of range");
        assert!(parse_ranking(r#"{"ranking":[0,1,2],"reason":""}"#, 3).is_err(), "zero-based");
        assert!(parse_ranking(r#"{"reason":"x"}"#, 3).is_err());
        let fenced = "```json\n{\"ranking\":[3,1,2],\"reason\":\"ok\"}\n```";
        assert_eq!(parse_ranking(fenced, 3).unwrap().0, vec![2, 0, 1]);
    }

    #[test]
    fn a_reason_that_names_a_different_image_than_the_pick_is_dropped() {
        // Real model output seen in testing: ranks image 3 first but praises "Image 1".
        let contradictory = r#"{"ranking":[3,2,1],"reason":"Image 1 has the most detail and depth."}"#;
        assert_eq!(parse_ranking(contradictory, 3).unwrap().1, "");

        let consistent = r#"{"ranking":[3,2,1],"reason":"Image 3 has the best expression."}"#;
        assert_eq!(parse_ranking(consistent, 3).unwrap().1, "Image 3 has the best expression.");

        let no_number = r#"{"ranking":[2,1,3],"reason":"Soft light and a relaxed pose."}"#;
        assert_eq!(parse_ranking(no_number, 3).unwrap().1, "Soft light and a relaxed pose.");

        // Only the first mention counts: comparing against another frame is fine.
        let comparing = r#"{"ranking":[2,1,3],"reason":"Photo 2 beats photo 1 on expression."}"#;
        assert!(!parse_ranking(comparing, 3).unwrap().1.is_empty());
    }

    #[test]
    fn two_passes_cancel_out_a_bias_towards_the_first_image() {
        // A model with pure positional bias says "image 1, then 2, then 3" in both orders.
        // Forward that favours frame 0, reversed it favours frame 2: every frame ends level.
        let (_, points) = combine_passes(3, &[0, 1, 2], &[0, 1, 2]);
        assert_eq!(points, vec![4.0, 4.0, 4.0]);

        // A real preference for frame 1 survives both orders (it is image 2 when reversed too).
        let (order, points) = combine_passes(3, &[1, 0, 2], &[1, 2, 0]);
        assert_eq!(order[0], 1);
        assert!(points[1] > points[0] && points[1] > points[2]);
    }

    #[test]
    fn series_request_carries_every_frame_in_one_message() {
        let reply = wrap(r#"{"ranking":[3,1,2],"reason":"Best expression."}"#);
        let (endpoint, received) = mock_ollama(200, reply);
        let imgs: Vec<String> = ["QQ==", "Qg==", "Qw=="].iter().map(|s| s.to_string()).collect();
        let (order, reason) = rank_pass(&endpoint, "gemma3:4b", &imgs).unwrap();
        assert_eq!(order, vec![2, 0, 1]);
        assert_eq!(reason, "Best expression.");

        let sent: Value = serde_json::from_str(&received.recv().unwrap()).unwrap();
        assert_eq!(sent["messages"][0]["images"].as_array().unwrap().len(), 3);
        assert_eq!(sent["messages"][0]["images"][2], "Qw==");
        assert!(sent["messages"][0]["content"].as_str().unwrap().contains("numbered 1 to 3"));
        assert!(sent["format"]["required"].as_array().unwrap().iter().any(|k| k == "ranking"));
    }

    #[test]
    fn only_local_http_endpoints_are_accepted() {
        for ok in ["http://localhost:11434", "http://127.0.0.1:11434/", "http://[::1]:11434", "http://LOCALHOST"] {
            assert!(validate_endpoint(ok).is_ok(), "{ok} should be allowed");
        }
        for bad in [
            "https://localhost:11434",          // TLS is not even compiled in
            "http://example.com",
            "http://192.168.1.20:11434",        // another machine on the LAN
            "http://localhost.evil.com",
            "http://127.0.0.1@evil.com",
            "http://user:pw@localhost",
            "ftp://localhost",
            "localhost:11434",
        ] {
            assert!(validate_endpoint(bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn parses_a_well_formed_reply_and_clamps_scores() {
        let r = parse_reply(GOOD, "m").unwrap();
        assert_eq!((r.composition, r.subject, r.people, r.impact), (3.5, 4.0, Some(2.5), 3.0));

        let wild = r#"{"composition":9,"subject":-2,"people_present":false,"people":4,"impact":2,"summary":"x"}"#;
        let r = parse_reply(wild, "m").unwrap();
        assert_eq!((r.composition, r.subject), (5.0, 0.0));
        assert_eq!(r.people, None, "people score is ignored when nobody is visible");
    }

    #[test]
    fn tolerates_code_fences_and_rejects_garbage() {
        let fenced = format!("Sure!\n```json\n{GOOD}\n```");
        assert!(parse_reply(&fenced, "m").is_ok());
        assert!(parse_reply("no json here", "m").is_err());
        assert!(parse_reply(r#"{"composition":3}"#, "m").unwrap_err().contains("subject"));
    }

    #[test]
    fn sends_the_image_and_schema_and_reads_the_answer() {
        let (endpoint, received) = mock_ollama(200, wrap(GOOD));
        let r = review_image(&endpoint, "gemma3:4b", "QUJD").unwrap();
        assert_eq!(r.composition, 3.5);

        let sent: Value = serde_json::from_str(&received.recv().unwrap()).unwrap();
        assert_eq!(sent["model"], "gemma3:4b");
        assert_eq!(sent["stream"], false);
        assert_eq!(sent["messages"][0]["images"][0], "QUJD");
        assert!(sent["format"]["required"].as_array().unwrap().iter().any(|k| k == "composition"));
        assert_eq!(sent["options"]["temperature"], 0);
    }

    #[test]
    fn surfaces_ollamas_own_error_message() {
        let (endpoint, _rx) = mock_ollama(404, r#"{"error":"model 'nope' not found"}"#.to_string());
        let err = review_image(&endpoint, "nope", "QUJD").unwrap_err();
        assert!(err.contains("404") && err.contains("not found"), "got: {err}");
    }

    #[test]
    fn check_lists_installed_models_and_reports_a_dead_server() {
        let tags = json!({"models": [{"name": "gemma3:4b", "size": 3300000000u64}]}).to_string();
        let (endpoint, _rx) = mock_ollama(200, tags);
        let status = ai_check(endpoint);
        assert!(status.reachable);
        assert_eq!(status.models[0].name, "gemma3:4b");

        let dead = ai_check("http://127.0.0.1:1".to_string());
        assert!(!dead.reachable && dead.error.is_some());
        assert!(!ai_check("http://example.com".to_string()).reachable);
    }
}

//! Re-back on the tools page: an old recording's own vocals over a new backing
//! track. The original is split (or its stems are taken where it has them),
//! its instrumental cover - made beforehand with Create - Cover, Instrumental -
//! is tuned, time-warped to the original's drifting tempo and stretched, and the
//! original vocal stem goes over it untouched. The result is a new library
//! track made from the original.

use std::sync::atomic::Ordering;

use anyhow::Context;
use audio_post::reback::{self, Report, Settings, Stage};
use audio_post::Stereo;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{api_error, ApiError, AppState};

pub const TOOL: &str = "reback";

/// One re-back, at work or the last one, as the interface sees it.
#[derive(Debug, Clone, Serialize)]
pub struct RebackRun {
    pub song_id: String,
    pub cover_id: String,
    pub title: String,
    /// stems, tuning, aligning, stretching, mixing, saving, done, stopped
    pub stage: &'static str,
    /// When the run began, in milliseconds since the Unix epoch.
    pub started_ms: u64,
    pub done: bool,
    pub error: Option<String>,
    /// What the run found: the tuning offset, the speeds, the time map.
    pub report: Option<Report>,
    /// The library track the result became.
    pub library_song: Option<String>,
}

/// What the cover is aligned to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignOn {
    /// The original without its vocals, from its stems: the cover has no voice either.
    #[default]
    Stems,
    /// The whole original mix.
    Mix,
}

#[derive(Debug, Deserialize)]
pub struct RebackRequest {
    /// The original recording, a library track.
    pub song_id: String,
    /// Its instrumental cover, a library track.
    pub cover_id: String,
    #[serde(default)]
    pub align_on: AlignOn,
    #[serde(flatten)]
    pub settings: Settings,
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Tuning => "tuning",
        Stage::Aligning => "aligning",
        Stage::Stretching => "stretching",
        Stage::Mixing => "mixing",
    }
}

/// The re-back at work or the last one, and whether a track without stems could be split here.
pub async fn status(State(state): State<AppState>) -> Json<Value> {
    let runtime = state.separation_config.read().await.runtime;
    Json(serde_json::json!({
        "run": state.reback_run.read().await.clone(),
        "separator_ready": crate::htdemucs_installed(&state, runtime),
    }))
}

pub async fn cancel(State(state): State<AppState>) -> Json<Value> {
    state.reback_stop.store(true, Ordering::Relaxed);
    Json(serde_json::json!({ "cancelled": true }))
}

fn song(state: &AppState, id: &str) -> Result<crate::library::Song, (StatusCode, Json<ApiError>)> {
    let song = state
        .library
        .get_song(id)
        .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, format!("Song {id} not found")))?;
    crate::refuse_trashed(&song)?;
    Ok(song)
}

fn interleaved(samples: &[f32], rate: u32) -> Stereo {
    let (left, right) = samples.chunks_exact(2).map(|frame| (frame[0], frame[1])).unzip();
    Stereo::new(left, right, rate)
}

/// Adds `more` into `sum`, sample by sample.
fn add_into(sum: &mut Option<Stereo>, more: Stereo) {
    match sum {
        None => *sum = Some(more),
        Some(sum) => {
            for (a, b) in sum.left.iter_mut().zip(&more.left) {
                *a += b;
            }
            for (a, b) in sum.right.iter_mut().zip(&more.right) {
                *a += b;
            }
        }
    }
}

/// The stems that make the original without its voice, of those on disk: Mel-Band RoFormer's instrumental, or
/// every HT-Demucs stem but the vocals.
pub fn guide_stems(on_disk: &[String]) -> Vec<String> {
    if on_disk.iter().any(|stem| stem == "instrumental") {
        return vec!["instrumental".into()];
    }
    on_disk.iter().filter(|stem| crate::separation::STEMS.contains(&stem.as_str()) && *stem != "vocals").cloned().collect()
}

/// Re-backs a library track with its instrumental cover, in the background: the result becomes a new library track
/// beside it. Re-backing the same track with the same cover again replaces the track the last one made.
pub async fn start(State(state): State<AppState>, Json(request): Json<RebackRequest>) -> Result<Json<Value>, (StatusCode, Json<ApiError>)> {
    crate::card_free_of_training(&state, "re-back tracks").await?;
    let mut slot = state.reback_run.write().await;
    if slot.as_ref().is_some_and(|run| !run.done) {
        return Err(api_error(StatusCode::CONFLICT, "a track is already being re-backed".into()));
    }
    let original = song(&state, request.song_id.trim())?;
    let cover = song(&state, request.cover_id.trim())?;
    if original.id == cover.id {
        return Err(api_error(StatusCode::BAD_REQUEST, "choose the instrumental cover of the track, not the track itself".into()));
    }
    if let Some((_, tool)) = crate::derived_from(&original)
        && (tool == "stems" || tool == TOOL)
    {
        return Err(api_error(StatusCode::CONFLICT, format!("this track was made by {tool}; re-back the recording it came from")));
    }
    let original_audio = state.library.media_path_for_song(&original).ok_or_else(|| api_error(StatusCode::BAD_REQUEST, format!("{} has no stored audio", original.title)))?;
    let cover_audio = state.library.media_path_for_song(&cover).ok_or_else(|| api_error(StatusCode::BAD_REQUEST, format!("{} has no stored audio", cover.title)))?;
    let on_disk = crate::stems_on_disk(&state, &original.id);
    let split = !on_disk.iter().any(|stem| stem == "vocals");
    let runtime = state.separation_config.read().await.runtime;
    let build = if split {
        if state.separation_run.read().await.as_ref().is_some_and(|run| !run.done) {
            return Err(api_error(StatusCode::CONFLICT, "a track is being split into stems; re-back when it is done".into()));
        }
        if !crate::htdemucs_installed(&state, runtime) {
            return Err(api_error(StatusCode::BAD_REQUEST, "this track has no vocal stem yet: split it into stems first, or install HT-Demucs (Settings - Models - Stems)".into()));
        }
        Some(crate::separation_build(runtime).map_err(|error| api_error(StatusCode::BAD_REQUEST, format!("{error:#}")))?)
    } else {
        None
    };
    state.reback_stop.store(false, Ordering::Relaxed);
    *slot = Some(RebackRun {
        song_id: original.id.clone(),
        cover_id: cover.id.clone(),
        title: original.title.clone(),
        stage: if split { "stems" } else { "tuning" },
        started_ms: crate::unix_millis(),
        done: false,
        error: None,
        report: None,
        library_song: None,
    });
    drop(slot);

    let background = state.clone();
    let wanted = state.separation_config.read().await.stems.clone();
    let song_id = original.id.clone();
    let reply_id = original.id.clone();
    tokio::task::spawn_blocking(move || {
        let stop = || background.reback_stop.load(Ordering::Relaxed);
        let set_stage = |stage: &'static str| {
            if let Some(run) = background.reback_run.blocking_write().as_mut() {
                run.stage = stage;
            }
        };
        let outcome = (|| -> anyhow::Result<Option<(String, Report, Option<String>)>> {
            let (vocals, guide) = match build {
                Some(build) => {
                    let audio = crate::audio_pcm::decode_stereo_44k(&original_audio)?;
                    let separated = crate::separation::separate_htdemucs(&background.acpp, &background.separator.model_path(), &audio, build, &stop)?;
                    if stop() {
                        return Ok(None);
                    }
                    let mut written = Vec::new();
                    let (mut vocals, mut guide) = (None, None);
                    for stem in separated.stems {
                        if wanted.iter().any(|name| name == stem.name) {
                            crate::separation::write_wav_stereo(&crate::stem_path(&background, &song_id, stem.name), &stem.samples)?;
                            written.push(stem.name.to_string());
                        }
                        let audio = interleaved(&stem.samples, 44_100);
                        if stem.name == "vocals" { vocals = Some(audio) } else { add_into(&mut guide, audio) }
                    }
                    // the stems are a result like any other: they go into the library beside the song
                    if let Err(error) = crate::stems_into_library(&background, &song_id, &written, "HT-Demucs") {
                        crate::studio_log!("[ERROR] the stems of {song_id} made for re-back did not reach the library: {error:#}");
                    }
                    (vocals.context("HT-Demucs gave no vocal stem")?, guide)
                }
                None => {
                    let vocals = crate::audio_pcm::decode_stereo(&crate::stem_path(&background, &song_id, "vocals"))?;
                    let mut guide = None;
                    if request.align_on == AlignOn::Stems {
                        for stem in guide_stems(&on_disk) {
                            add_into(&mut guide, crate::audio_pcm::decode_stereo(&crate::stem_path(&background, &song_id, &stem))?.resampled(vocals.rate)?);
                        }
                    }
                    (vocals, guide)
                }
            };
            let guide = match (request.align_on, guide) {
                (AlignOn::Stems, Some(guide)) => guide,
                _ => crate::audio_pcm::decode_stereo(&original_audio)?,
            };
            let cover_audio = crate::audio_pcm::decode_stereo(&cover_audio)?;
            let Some((mix, report)) = reback::reback(&vocals, &guide, &cover_audio, &request.settings, &mut |stage| set_stage(stage_name(stage)), &stop)? else {
                return Ok(None);
            };
            set_stage("saving");
            let audio = crate::audio_pcm::wav24_bytes(&mix)?;
            let (id, problem) = into_library(&background, &original, &cover, &request, &report, audio)?;
            Ok(Some((id, report, problem)))
        })();
        let stopped = stop();
        let handle = tokio::runtime::Handle::current();
        handle.spawn(async move {
            match &outcome {
                Ok(Some(_)) => crate::work_ended(&background, TOOL, Ok(()), false),
                Ok(None) => crate::work_ended(&background, TOOL, Err("stopped"), true),
                Err(error) => crate::work_ended(&background, TOOL, Err(&format!("{error:#}")), stopped),
            }
            if let Some(run) = background.reback_run.write().await.as_mut() {
                run.done = true;
                match outcome {
                    Ok(Some((id, report, problem))) => {
                        run.stage = "done";
                        run.library_song = Some(id);
                        run.report = Some(report);
                        run.error = problem;
                    }
                    Ok(None) => run.stage = "stopped",
                    Err(_) if stopped => run.stage = "stopped",
                    Err(error) => run.error = Some(format!("{error:#}")),
                }
            }
        });
    });
    Ok(Json(serde_json::json!({ "started": true, "song_id": reply_id })))
}

/// The re-backed audio as a library track made from the original: its title marked, its words and its cover. It
/// replaces the track an earlier re-back of the same original with the same cover made.
fn into_library(
    state: &AppState,
    original: &crate::library::Song,
    cover: &crate::library::Song,
    request: &RebackRequest,
    report: &Report,
    audio: Vec<u8>,
) -> anyhow::Result<(String, Option<String>)> {
    let duration = crate::library::audio_duration_seconds(&audio, "wav", None);
    let settings = serde_json::json!({
        "cover_id": cover.id,
        "cover_title": cover.title,
        "align_on": request.align_on,
        "vocal_gain_db": request.settings.vocal_gain_db,
        "backing_gain_db": request.settings.backing_gain_db,
        "tuning_cents": report.tuning_cents,
        "tuning_measured": report.tuning_measured,
        "slowest": report.slowest,
        "fastest": report.fastest,
    });
    let song = state
        .library
        .import_audio_song(crate::library::AudioImportInput {
            title: format!("{} · re-backed", original.title),
            caption: if cover.caption.trim().is_empty() { original.caption.clone() } else { cover.caption.clone() },
            lyrics: original.lyrics.clone(),
            metadata: serde_json::json!({ "derived": crate::derivation(original, TOOL, settings), "duration_seconds": duration }),
            generation_settings: Value::Null,
            engine_id: TOOL.into(),
            profile_id: None,
            source: TOOL.into(),
            audio_extension: "wav".into(),
            audio,
        })?
        .song;
    crate::cover_like(state, original, &song.id)?;
    let replaced: Vec<crate::library::Song> = state
        .library
        .list_songs()?
        .into_iter()
        .filter(|old| {
            old.id != song.id
                && crate::derived_from(old) == Some((original.id.as_str(), TOOL))
                && old.metadata.pointer("/derived/settings/cover_id").and_then(Value::as_str) == Some(cover.id.as_str())
        })
        .collect();
    let mut kept = Vec::new();
    for old in replaced {
        let files = crate::song_files(state, &old);
        state.library.delete_song(&old.id)?;
        for path in files {
            if let Err(error) = std::fs::remove_file(&path) {
                kept.push(format!("{}: {error}", path.display()));
            }
        }
    }
    let problem = (!kept.is_empty()).then(|| format!("the re-backed track is in the library, but files of the earlier one could not be removed: {}", kept.join("; ")));
    Ok((song.id, problem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_is_the_original_without_its_voice() {
        let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(guide_stems(&names(&["drums", "bass", "other", "vocals", "guitar", "piano"])), names(&["drums", "bass", "other", "guitar", "piano"]));
        assert_eq!(guide_stems(&names(&["vocals", "instrumental", "bass"])), names(&["instrumental"]), "RoFormer's instrumental is the whole band at once");
        assert!(guide_stems(&names(&["vocals"])).is_empty(), "nothing but the voice: the mix is the guide");
    }

    #[test]
    fn a_request_takes_the_settings_beside_the_tracks() {
        let request: RebackRequest = serde_json::from_value(serde_json::json!({ "song_id": "a", "cover_id": "b", "vocal_gain_db": 2.0, "align_on": "mix" })).unwrap();
        assert_eq!(request.align_on, AlignOn::Mix);
        assert_eq!(request.settings.vocal_gain_db, 2.0);
        assert!(request.settings.tune, "measuring the tuning is the default");
        let bare: RebackRequest = serde_json::from_value(serde_json::json!({ "song_id": "a", "cover_id": "b" })).unwrap();
        assert_eq!(bare.align_on, AlignOn::Stems);
    }
}

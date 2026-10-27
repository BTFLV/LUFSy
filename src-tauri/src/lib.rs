pub mod analysis;
pub mod decode;
pub mod kweight;
pub mod limiter;
pub mod plan;
pub mod render;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use analysis::{Analysis, AnalysisDto};
use plan::{Params, PlanDto};
use render::{Format, Report};

#[derive(Default)]
struct AppState {
    analysis: Mutex<Option<Arc<Analysis>>>,
    cancel: Arc<AtomicBool>,
}

#[derive(Serialize, Clone)]
struct Progress {
    phase: &'static str,
    value: f64,
}

const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "flac", "mp3", "m4a", "aac", "mp4", "ogg", "oga",
];

fn current(state: &State<'_, AppState>) -> Result<Arc<Analysis>, String> {
    state
        .analysis
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Noch kein Set geladen".into())
}

#[tauri::command]
async fn pick_input(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Set öffnen")
        .add_filter("Audio", AUDIO_EXTENSIONS)
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn pick_output(app: AppHandle, default_name: String, extension: String) -> Option<String> {
    let label = if extension == "mp3" { "MP3" } else { "WAV" };
    app.dialog()
        .file()
        .set_title("Export speichern")
        .set_file_name(default_name)
        .add_filter(label, &[extension.as_str()])
        .blocking_save_file()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn analyze(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<AnalysisDto, String> {
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let progress_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let a = analysis::analyze(
            &PathBuf::from(&path),
            |v| {
                let _ = progress_app.emit(
                    "progress",
                    Progress {
                        phase: "analyze",
                        value: v,
                    },
                );
            },
            || cancel.load(Ordering::SeqCst),
        )?;
        let dto = a.to_dto();
        Ok::<_, String>((a, dto))
    })
    .await
    .map_err(|e| e.to_string())??;

    *state.analysis.lock().unwrap() = Some(Arc::new(result.0));
    Ok(result.1)
}

#[tauri::command]
async fn plan(state: State<'_, AppState>, params: Params) -> Result<PlanDto, String> {
    let a = current(&state)?;
    tauri::async_runtime::spawn_blocking(move || plan::compute(&a, &params).1)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn export(
    app: AppHandle,
    state: State<'_, AppState>,
    params: Params,
    format: Format,
    path: String,
) -> Result<Report, String> {
    let a = current(&state)?;
    let same = |p: &str| std::fs::canonicalize(p).ok();
    if same(&path).is_some() && same(&path) == same(&a.path) {
        return Err("Das ist die Originaldatei. Bitte unter einem anderen Namen speichern.".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (plan, _) = plan::compute(&a, &params);
        render::render(
            &a,
            &plan,
            &params,
            format,
            &PathBuf::from(path),
            |v| {
                let _ = app.emit(
                    "progress",
                    Progress {
                        phase: "export",
                        value: v,
                    },
                );
            },
            || cancel.load(Ordering::SeqCst),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A file passed on the command line ("Open with" on Windows, or a script).
#[tauri::command]
fn initial_path() -> Option<String> {
    std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .chain(std::env::var_os("LUFSY_OPEN").map(PathBuf::from))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn cancel(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            pick_input,
            pick_output,
            analyze,
            plan,
            export,
            cancel,
            initial_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running LUFSy");
}

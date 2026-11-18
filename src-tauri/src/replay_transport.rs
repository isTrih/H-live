//! Binary transport for allowlisted replay media, independent of WebView routing.
use crate::backend::Backend;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub(crate) async fn replay_media_fetch(
    state: State<'_, Arc<Backend>>,
    url: String,
    range_start: Option<u64>,
    range_end: Option<u64>,
) -> Result<tauri::ipc::Response, String> {
    let backend = state.inner().clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let _operation = backend.read_operation()?;
        let runtime = backend.get()?;
        tauri::async_runtime::block_on(runtime.replay_media_fetch(&url, range_start, range_end))
    })
        .await
        .map_err(|_| "录播媒体后台不可用".to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

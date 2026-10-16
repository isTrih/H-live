use pyo3::{
    create_exception,
    exceptions::{PyOSError, PyRuntimeError, PyValueError},
    prelude::*,
};
use qingkan_core::{
    archive::{ArchiveDeleteFilter, ArchiveError, ArchiveOptions, ArchiveStore, MessageRecord},
    export::{self, CancelToken, ExportError, ExportOptions},
};
use std::{path::PathBuf, time::Duration};

create_exception!(qingkan_native, ExportCancelled, PyRuntimeError);
create_exception!(qingkan_native, ExportDataError, PyValueError);
create_exception!(qingkan_native, ArchiveDeleteCancelled, PyRuntimeError);
fn archive_error(error: ArchiveError) -> PyErr {
    match error {
        ArchiveError::Invalid(text) => PyValueError::new_err(text),
        ArchiveError::DeleteCancelled => ArchiveDeleteCancelled::new_err(error.to_string()),
        ArchiveError::Io(error) => PyOSError::new_err(error.to_string()),
        error => PyRuntimeError::new_err(error.to_string()),
    }
}
fn export_error(error: ExportError) -> PyErr {
    match error {
        ExportError::Cancelled => ExportCancelled::new_err(error.to_string()),
        ExportError::Data(text) => ExportDataError::new_err(text),
        ExportError::Archive(error) => archive_error(error),
        ExportError::Io(error) => PyOSError::new_err(error.to_string()),
        error => PyRuntimeError::new_err(error.to_string()),
    }
}
#[pyclass(name = "ArchiveStore", module = "qingkan_native")]
struct PyArchiveStore {
    store: ArchiveStore,
}
#[pymethods]
impl PyArchiveStore {
    #[new]
    #[pyo3(signature=(data_dir,queue_capacity=4096,batch_size=256,batch_wait_seconds=0.004))]
    fn new(
        py: Python<'_>,
        data_dir: PathBuf,
        queue_capacity: usize,
        batch_size: usize,
        batch_wait_seconds: f64,
    ) -> PyResult<Self> {
        if !batch_wait_seconds.is_finite() || batch_wait_seconds < 0.0 || batch_wait_seconds > 60.0
        {
            return Err(PyValueError::new_err("归档批次等待参数无效"));
        }
        let options = ArchiveOptions {
            queue_capacity,
            batch_size,
            batch_wait: Duration::from_secs_f64(batch_wait_seconds),
        };
        py.detach(move || ArchiveStore::open(data_dir, options).map(|store| Self { store }))
            .map_err(archive_error)
    }
    #[pyo3(signature=(record_json,title=String::new(),started_at=None))]
    fn record_message(
        &self,
        py: Python<'_>,
        record_json: String,
        title: String,
        started_at: Option<String>,
    ) -> PyResult<bool> {
        py.detach(|| {
            let record: MessageRecord =
                serde_json::from_str(&record_json).map_err(ArchiveError::from)?;
            self.store.record_message(record, title, started_at)
        })
        .map_err(archive_error)
    }
    fn metrics(&self, py: Python<'_>) -> String {
        py.detach(|| self.store.metrics().to_string())
    }
    fn recent_messages(&self, py: Python<'_>, params_json: String) -> PyResult<String> {
        py.detach(move || {
            let params = serde_json::from_str(&params_json)?;
            self.store.recent_messages(params).map(|v| v.to_string())
        })
        .map_err(archive_error)
    }
    #[pyo3(signature=(action,params_json,cancel_token=None))]
    fn quality(
        &self,
        py: Python<'_>,
        action: String,
        params_json: String,
        cancel_token: Option<PyRef<'_, PyCancelToken>>,
    ) -> PyResult<String> {
        let token = cancel_token.map(|v| v.token.clone()).unwrap_or_default();
        py.detach(move || {
            let params = serde_json::from_str(&params_json)?;
            self.store
                .quality_request(&action, params, token)
                .map(|v| v.to_string())
        })
        .map_err(archive_error)
    }
    fn load_monitor_duration(
        &self,
        py: Python<'_>,
        anchor_id: String,
        session_id: String,
    ) -> PyResult<f64> {
        py.detach(|| self.store.load_monitor_duration(&anchor_id, &session_id))
            .map_err(archive_error)
    }
    fn save_monitor_duration(
        &self,
        py: Python<'_>,
        anchor_id: String,
        session_id: String,
        seconds: f64,
    ) -> PyResult<()> {
        py.detach(|| {
            self.store
                .save_monitor_duration(&anchor_id, &session_id, seconds)
        })
        .map_err(archive_error)
    }
    fn list_anchors(&self, py: Python<'_>) -> PyResult<String> {
        py.detach(|| {
            self.store
                .list_anchors()
                .and_then(|rows| serde_json::to_string(&rows).map_err(Into::into))
        })
        .map_err(archive_error)
    }
    #[pyo3(signature=(selection_json,limit=200,offset=0))]
    fn list_sessions(
        &self,
        py: Python<'_>,
        selection_json: String,
        limit: usize,
        offset: usize,
    ) -> PyResult<String> {
        py.detach(|| {
            let selection: ArchiveDeleteFilter = serde_json::from_str(&selection_json)?;
            self.store
                .list_sessions(&selection, limit, offset)
                .map(|value| value.to_string())
        })
        .map_err(archive_error)
    }
    fn preview_delete(&self, py: Python<'_>, selection_json: String) -> PyResult<String> {
        py.detach(|| {
            let selection: ArchiveDeleteFilter = serde_json::from_str(&selection_json)?;
            self.store
                .preview_delete(&selection)
                .map(|value| value.to_string())
        })
        .map_err(archive_error)
    }
    #[pyo3(signature=(selection_json,cancel_token=None))]
    fn execute_delete(
        &self,
        py: Python<'_>,
        selection_json: String,
        cancel_token: Option<PyRef<'_, PyCancelToken>>,
    ) -> PyResult<String> {
        let token = cancel_token
            .map(|value| value.token.clone())
            .unwrap_or_default();
        py.detach(move || {
            let selection: ArchiveDeleteFilter = serde_json::from_str(&selection_json)?;
            self.store
                .execute_delete_cancellable(&selection, move || token.is_cancelled())
                .map(|value| value.to_string())
        })
        .map_err(archive_error)
    }
    #[pyo3(signature=(anchor_ids=None,month=None,types=None))]
    fn count_missing_time(
        &self,
        py: Python<'_>,
        anchor_ids: Option<Vec<String>>,
        month: Option<String>,
        types: Option<Vec<String>>,
    ) -> PyResult<u64> {
        py.detach(|| {
            self.store.snapshot()?.count_missing_time(
                anchor_ids.as_deref(),
                month.as_deref(),
                types.as_deref(),
            )
        })
        .map_err(archive_error)
    }
    #[pyo3(signature=(timeout=15.0))]
    fn close(&self, py: Python<'_>, timeout: f64) -> PyResult<bool> {
        if !timeout.is_finite() || !(0.0..=86400.0).contains(&timeout) {
            return Err(PyValueError::new_err("关闭等待时间无效"));
        }
        Ok(py.detach(|| self.store.close(Duration::from_secs_f64(timeout))))
    }
}
#[pyclass(name = "CancelToken", module = "qingkan_native", skip_from_py_object)]
#[derive(Clone, Default)]
struct PyCancelToken {
    token: CancelToken,
}
#[pymethods]
impl PyCancelToken {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn set(&self) {
        self.token.cancel();
    }
    fn cancel(&self) {
        self.token.cancel();
    }
    fn is_set(&self) -> bool {
        self.token.is_cancelled()
    }
}
#[pyfunction]
#[pyo3(signature=(data_dir,anchor_ids,month,output_path,format,types,cancel_token=None))]
fn export_month(
    py: Python<'_>,
    data_dir: PathBuf,
    anchor_ids: Vec<String>,
    month: String,
    output_path: PathBuf,
    format: String,
    types: Vec<String>,
    cancel_token: Option<PyRef<'_, PyCancelToken>>,
) -> PyResult<String> {
    let token = cancel_token
        .map(|value| value.token.clone())
        .unwrap_or_default();
    let options = ExportOptions {
        anchor_ids,
        month,
        output_path,
        format,
        types,
    };
    py.detach(move || export::export_month(&data_dir, options, &token))
        .map_err(export_error)
        .and_then(|report| {
            serde_json::to_string(&report).map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
}
pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyArchiveStore>()?;
    module.add_class::<PyCancelToken>()?;
    module.add("ExportCancelled", module.py().get_type::<ExportCancelled>())?;
    module.add("ExportDataError", module.py().get_type::<ExportDataError>())?;
    module.add(
        "ArchiveDeleteCancelled",
        module.py().get_type::<ArchiveDeleteCancelled>(),
    )?;
    module.add_function(wrap_pyfunction!(export_month, module)?)?;
    module.add_function(wrap_pyfunction!(export_quality, module)?)?;
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(data_dir,anchor_id,month,output_path,format,cancel_token=None))]
fn export_quality(
    py: Python<'_>,
    data_dir: PathBuf,
    anchor_id: String,
    month: String,
    output_path: PathBuf,
    format: String,
    cancel_token: Option<PyRef<'_, PyCancelToken>>,
) -> PyResult<String> {
    let token = cancel_token.map(|v| v.token.clone()).unwrap_or_default();
    py.detach(move || {
        export::export_quality(&data_dir, &anchor_id, &month, &output_path, &format, &token)
    })
    .map_err(export_error)
    .map(|v| v.to_string())
}

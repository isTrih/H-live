//! One FFI call per WebSocket frame; no Python objects are touched while the
//! native decoder/JSON encoder runs without the GIL.
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use qingkan_core::protocol;

#[pyfunction]
#[pyo3(signature = (data, fallback_room_id=""))]
fn decode_frame<'py>(
    py: Python<'py>,
    data: &[u8],
    fallback_room_id: &str,
) -> PyResult<(Option<Bound<'py, PyBytes>>, String)> {
    let result = py
        .detach(|| {
            let frame = protocol::decode_frame(data, fallback_room_id)?;
            let events = serde_json::to_string(&frame.events)
                .map_err(|_| protocol::ProtocolError::InvalidResponse)?;
            Ok::<_, protocol::ProtocolError>((frame.ack, events))
        })
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok((
        result.0.as_deref().map(|ack| PyBytes::new(py, ack)),
        result.1,
    ))
}

#[pyfunction]
#[pyo3(signature = (method, payload, fallback_room_id=""))]
fn decode_message(
    py: Python<'_>,
    method: &str,
    payload: &[u8],
    fallback_room_id: &str,
) -> PyResult<String> {
    py.detach(|| {
        let event = protocol::decode_message(method, payload, fallback_room_id)?;
        serde_json::to_string(&event).map_err(|_| protocol::ProtocolError::InvalidMessage)
    })
    .map_err(|error| PyValueError::new_err(error.to_string()))
}

#[pyfunction]
fn hb_frame(py: Python<'_>) -> Bound<'_, PyBytes> {
    PyBytes::new(py, &protocol::hb_frame())
}

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(decode_frame, module)?)?;
    module.add_function(wrap_pyfunction!(decode_message, module)?)?;
    module.add_function(wrap_pyfunction!(hb_frame, module)?)?;
    Ok(())
}

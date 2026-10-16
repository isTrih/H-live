use pyo3::prelude::*;

mod archive_bindings;
mod protocol_bindings;

#[pymodule]
fn qingkan_native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    archive_bindings::register(module)?;
    protocol_bindings::register(module)?;
    Ok(())
}

use pyo3::prelude::*;

#[pyfunction]
fn echo(py: Python<'_>, text: String) -> PyResult<Bound<'_, PyAny>> {
    let value: serde_json::Value = serde_json::json!({ "text": text });
    let out = py.detach(|| value);
    Ok(pythonize::pythonize(py, &out)?)
}

#[pymodule]
fn spike_pyo3(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(echo, m)?)
}

use std::io::{self, BufReader, BufWriter, Read, Write};

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};

pyo3::create_exception!(_phig, PhigError, pyo3::exceptions::PyException);

fn value_to_py(py: Python<'_>, val: &phig::Value) -> PyResult<PyObject> {
    Ok(match val {
        phig::Value::String(s) => PyString::new(py, s).into_any().unbind(),
        phig::Value::List(items) => {
            let py_items: Vec<PyObject> = items
                .iter()
                .map(|v| value_to_py(py, v))
                .collect::<PyResult<_>>()?;
            PyList::new(py, py_items)?.into_any().unbind()
        }
        phig::Value::Map(pairs) => {
            let dict = PyDict::new(py);
            for (k, v) in pairs {
                dict.set_item(k, value_to_py(py, v)?)?;
            }
            dict.into_any().unbind()
        }
    })
}

fn py_to_value(obj: &Bound<'_, PyAny>) -> PyResult<phig::Value> {
    Ok(if let Ok(dict) = obj.downcast::<PyDict>() {
        let mut pairs = Vec::new();
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            pairs.push((key, py_to_value(&v)?));
        }
        phig::Value::Map(pairs)
    } else if let Ok(list) = obj.downcast::<PyList>() {
        let items = list
            .iter()
            .map(|item| py_to_value(&item))
            .collect::<PyResult<Vec<_>>>()?;
        phig::Value::List(items)
    } else if let Ok(s) = obj.extract::<String>() {
        phig::Value::String(s)
    } else {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "unsupported type: {}",
            obj.get_type().qualname()?
        )));
    })
}

/// Adapts a Python text file object (with a `.read(size)` method) into `std::io::Read`.
struct PyReader<'py> {
    fp: Bound<'py, PyAny>,
}

impl<'py> Read for PyReader<'py> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let chunk: String = self
            .fp
            .call_method1("read", (buf.len(),))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
            .extract()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        let bytes = chunk.as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}

/// Adapts a Python text file object (with a `.write(s)` method) into `std::io::Write`.
struct PyWriter<'py> {
    fp: Bound<'py, PyAny>,
}

impl<'py> Write for PyWriter<'py> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s =
            std::str::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.fp
            .call_method1("write", (s,))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.fp.call_method0("flush");
        Ok(())
    }
}

fn to_py_err(e: phig::Error) -> PyErr {
    PyErr::new::<PhigError, _>(e.to_string())
}

#[pyfunction]
fn load(py: Python<'_>, fp: Bound<'_, PyAny>) -> PyResult<PyObject> {
    let reader = BufReader::new(PyReader { fp });
    let value: phig::Value = phig::from_reader(reader).map_err(to_py_err)?;
    value_to_py(py, &value)
}

#[pyfunction]
fn loads(py: Python<'_>, s: &str) -> PyResult<PyObject> {
    let value: phig::Value = phig::from_str(s).map_err(to_py_err)?;
    value_to_py(py, &value)
}

#[pyfunction]
fn dump(obj: &Bound<'_, PyAny>, fp: Bound<'_, PyAny>) -> PyResult<()> {
    let value = py_to_value(obj)?;
    let mut writer = BufWriter::new(PyWriter { fp });
    phig::to_writer(&value, &mut writer).map_err(to_py_err)?;
    writer
        .flush()
        .map_err(|e| PyErr::new::<PhigError, _>(e.to_string()))
}

#[pyfunction]
fn dumps(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let value = py_to_value(obj)?;
    phig::to_string(&value).map_err(to_py_err)
}

#[pymodule]
fn _phig(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(load, m)?)?;
    m.add_function(wrap_pyfunction!(loads, m)?)?;
    m.add_function(wrap_pyfunction!(dump, m)?)?;
    m.add_function(wrap_pyfunction!(dumps, m)?)?;
    m.add("PhigError", m.py().get_type::<PhigError>())?;
    Ok(())
}

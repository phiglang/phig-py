use std::io::{self, BufReader, BufWriter, Read, Write};

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};

use phig::parse::{Event, Parser};

pyo3::create_exception!(_phig, PhigError, pyo3::exceptions::PyException);

/// Wrapper to stash a `PyErr` inside an `io::Error`.
struct PyErrWrapper(PyErr);

impl std::fmt::Debug for PyErrWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PyErrWrapper({:?})", self.0)
    }
}

impl std::fmt::Display for PyErrWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PyErrWrapper {}

enum PyPhigError {
    Phig(phig::Error),
    Python(PyErr),
}

impl From<phig::Error> for PyPhigError {
    fn from(e: phig::Error) -> Self {
        // If the error wraps an io::Error that contains a PyErr, extract it.
        if let phig::Error::Io(io_err) = e {
            if let Some(wrapper) = io_err.into_inner() {
                if let Ok(wrapper) = wrapper.downcast::<PyErrWrapper>() {
                    return PyPhigError::Python(wrapper.0);
                }
            }
            // Non-Python IO error — surface as PhigError
            return PyPhigError::Phig(phig::Error::new("IO error"));
        }
        PyPhigError::Phig(e)
    }
}

impl From<PyErr> for PyPhigError {
    fn from(e: PyErr) -> Self {
        PyPhigError::Python(e)
    }
}

impl From<PyPhigError> for PyErr {
    fn from(e: PyPhigError) -> Self {
        match e {
            PyPhigError::Phig(e) => PyErr::new::<PhigError, _>(e.to_string()),
            PyPhigError::Python(e) => e,
        }
    }
}

fn parse_to_pyobject(py: Python<'_>, reader: impl Read) -> Result<PyObject, PyPhigError> {
    enum Frame<'py> {
        Map {
            dict: Bound<'py, PyDict>,
            pending_key: Option<String>,
        },
        List {
            items: Vec<PyObject>,
        },
    }

    fn push_value<'py>(
        stack: &mut Vec<Frame<'py>>,
        result: &mut Option<PyObject>,
        value: PyObject,
    ) -> Result<(), PyErr> {
        match stack.last_mut() {
            Some(Frame::Map { dict, pending_key }) => {
                dict.set_item(pending_key.take().expect("map value without key"), value)?;
            }
            Some(Frame::List { items }) => items.push(value),
            None => *result = Some(value),
        }
        Ok(())
    }

    let mut parser = Parser::new(reader);
    let mut stack: Vec<Frame<'_>> = Vec::new();
    let mut result: Option<PyObject> = None;

    for event in &mut parser {
        let event = event?;
        match event {
            Event::StartMap => stack.push(Frame::Map {
                dict: PyDict::new(py),
                pending_key: None,
            }),
            Event::EndMap => {
                let Frame::Map { dict, .. } = stack.pop().expect("unbalanced EndMap") else {
                    panic!("EndMap on non-map frame");
                };
                push_value(&mut stack, &mut result, dict.into_any().unbind())?;
            }
            Event::StartList => stack.push(Frame::List { items: Vec::new() }),
            Event::EndList => {
                let Frame::List { items } = stack.pop().expect("unbalanced EndList") else {
                    panic!("EndList on non-list frame");
                };
                let list = PyList::new(py, items)?.into_any().unbind();
                push_value(&mut stack, &mut result, list)?;
            }
            Event::Key(k) => match stack.last_mut() {
                Some(Frame::Map { pending_key, .. }) => {
                    *pending_key = Some(k);
                }
                _ => panic!("Key outside of map"),
            },
            Event::String(s) => {
                let obj = PyString::new(py, &s).into_any().unbind();
                push_value(&mut stack, &mut result, obj)?;
            }
        }
    }

    Ok(result.expect("no value produced"))
}

fn walk_py_obj<W: io::Write>(
    obj: &Bound<'_, PyAny>,
    fmt: &mut phig::fmt::Formatter<W>,
) -> Result<(), PyPhigError> {
    if let Ok(dict) = obj.downcast::<PyDict>() {
        fmt.map_start()?;
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            fmt.key(key)?;
            walk_py_obj(&v, fmt)?;
        }
        fmt.map_end()?;
    } else if let Ok(list) = obj.downcast::<PyList>() {
        fmt.list_start()?;
        for item in list.iter() {
            walk_py_obj(&item, fmt)?;
        }
        fmt.list_end()?;
    } else if obj.hasattr("__dataclass_fields__")? {
        let fields = obj.getattr("__dataclass_fields__")?;
        let field_dict: &Bound<PyDict> = fields
            .downcast()
            .map_err(|e| PyPhigError::Python(e.into()))?;
        fmt.map_start()?;
        for (key_obj, _) in field_dict.iter() {
            let key: String = key_obj.extract()?;
            let value = obj.getattr(key.as_str())?;
            fmt.key(key)?;
            walk_py_obj(&value, fmt)?;
        }
        fmt.map_end()?;
    } else if let Ok(b) = obj.downcast::<PyBool>() {
        fmt.string(if b.is_true() { "true" } else { "false" }.to_string())?;
    } else if let Ok(s) = obj.extract::<String>() {
        fmt.string(s)?;
    } else if obj.is_instance_of::<PyInt>() || obj.is_instance_of::<PyFloat>() {
        let s: String = obj.str()?.extract()?;
        fmt.string(s)?;
    } else {
        return Err(PyPhigError::Python(PyErr::new::<
            pyo3::exceptions::PyTypeError,
            _,
        >(format!(
            "unsupported type: {}",
            obj.get_type().qualname()?
        ))));
    }
    Ok(())
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
            .map_err(|e| io::Error::new(io::ErrorKind::Other, PyErrWrapper(e)))?
            .extract()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, PyErrWrapper(e)))?;
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
            .map_err(|e| io::Error::new(io::ErrorKind::Other, PyErrWrapper(e)))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.fp
            .call_method0("flush")
            .map_err(|e| io::Error::new(io::ErrorKind::Other, PyErrWrapper(e)))?;
        Ok(())
    }
}

#[pyfunction]
fn load(py: Python<'_>, fp: Bound<'_, PyAny>) -> PyResult<PyObject> {
    let reader = BufReader::new(PyReader { fp });
    Ok(parse_to_pyobject(py, reader)?)
}

#[pyfunction]
fn loads(py: Python<'_>, s: &str) -> PyResult<PyObject> {
    Ok(parse_to_pyobject(py, s.as_bytes())?)
}

fn check_top_level(obj: &Bound<'_, PyAny>) -> PyResult<()> {
    if obj.downcast::<PyDict>().is_ok() || obj.hasattr("__dataclass_fields__")? {
        Ok(())
    } else {
        Err(PyErr::new::<PhigError, _>("top-level value must be a map"))
    }
}

#[pyfunction]
fn dump(obj: &Bound<'_, PyAny>, fp: Bound<'_, PyAny>) -> PyResult<()> {
    check_top_level(obj)?;
    let writer = BufWriter::new(PyWriter { fp });
    let mut fmt = phig::fmt::Formatter::new(writer);
    walk_py_obj(obj, &mut fmt)?;
    fmt.into_inner()
        .flush()
        .map_err(|e| PyPhigError::from(phig::Error::from(e)))?;
    Ok(())
}

#[pyfunction]
fn dumps(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    check_top_level(obj)?;
    let mut buf = Vec::new();
    {
        let mut fmt = phig::fmt::Formatter::new(&mut buf);
        walk_py_obj(obj, &mut fmt)?;
    }
    Ok(String::from_utf8(buf).expect("phig output is always valid UTF-8"))
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

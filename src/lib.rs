use std::io::{self, BufReader, BufWriter, Read, Write};

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};

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

struct PyHandler<'py> {
    py: Python<'py>,
    stack: Vec<PyBuildFrame<'py>>,
    result: Option<PyObject>,
}

enum PyBuildFrame<'py> {
    Map {
        dict: Bound<'py, PyDict>,
        pending_key: Option<String>,
    },
    List {
        items: Vec<PyObject>,
    },
}

impl<'py> PyHandler<'py> {
    fn new(py: Python<'py>) -> Self {
        PyHandler {
            py,
            stack: Vec::new(),
            result: None,
        }
    }

    fn finish(self) -> PyObject {
        self.result.expect("no value produced")
    }

    fn push_value(&mut self, value: PyObject) -> Result<(), PyPhigError> {
        match self.stack.last_mut() {
            Some(PyBuildFrame::Map { dict, pending_key }) => {
                let key = pending_key.take().expect("map value without key");
                dict.set_item(key, value)?;
            }
            Some(PyBuildFrame::List { items }) => {
                items.push(value);
            }
            None => {
                self.result = Some(value);
            }
        }
        Ok(())
    }
}

impl<'py> phig::parse::Handler for PyHandler<'py> {
    type Error = PyPhigError;

    fn map_start(&mut self) -> Result<(), PyPhigError> {
        self.stack.push(PyBuildFrame::Map {
            dict: PyDict::new(self.py),
            pending_key: None,
        });
        Ok(())
    }

    fn map_end(&mut self) -> Result<(), PyPhigError> {
        let frame = self.stack.pop().expect("unbalanced map_end");
        match frame {
            PyBuildFrame::Map { dict, .. } => self.push_value(dict.into_any().unbind()),
            _ => panic!("map_end on non-map frame"),
        }
    }

    fn list_start(&mut self) -> Result<(), PyPhigError> {
        self.stack.push(PyBuildFrame::List { items: Vec::new() });
        Ok(())
    }

    fn list_end(&mut self) -> Result<(), PyPhigError> {
        let frame = self.stack.pop().expect("unbalanced list_end");
        match frame {
            PyBuildFrame::List { items } => {
                let list = PyList::new(self.py, items)?.into_any().unbind();
                self.push_value(list)
            }
            _ => panic!("list_end on non-list frame"),
        }
    }

    fn key(&mut self, key: String) -> Result<(), PyPhigError> {
        match self.stack.last_mut() {
            Some(PyBuildFrame::Map { pending_key, .. }) => {
                *pending_key = Some(key);
            }
            _ => panic!("key outside of map"),
        }
        Ok(())
    }

    fn string(&mut self, value: String) -> Result<(), PyPhigError> {
        let py_str = PyString::new(self.py, &value).into_any().unbind();
        self.push_value(py_str)
    }
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
    let mut handler = PyHandler::new(py);
    phig::parse::parse_events(reader, &mut handler)?;
    Ok(handler.finish())
}

#[pyfunction]
fn loads(py: Python<'_>, s: &str) -> PyResult<PyObject> {
    let mut handler = PyHandler::new(py);
    phig::parse::parse_events(s.as_bytes(), &mut handler)?;
    Ok(handler.finish())
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

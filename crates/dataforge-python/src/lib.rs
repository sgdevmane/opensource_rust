// =============================================================================
// DataForge Python — PyO3 Bindings
// =============================================================================
// Exposes the high-performance spreadsheet engine to Python.
//
// Key features:
// - Non-blocking native iteration with GIL release (`py.allow_threads`)
// - Idiomatic Python iterator pattern (`__iter__` / `__next__`)
// - Zero-copy representations for JSON/Pandas conversion
// =============================================================================

use pyo3::prelude::*;
use pyo3::exceptions::PyRuntimeError;

use dataforge_core::config::{ReaderConfig, WriterConfig};
use dataforge_core::csv::CsvReader;
use dataforge_core::xlsx::XlsxReader;
use dataforge_core::ods::OdsReader;
use dataforge_core::types::CellValue;

/// Internal helper to map core errors to PyErr.
fn to_py_err(err: dataforge_core::error::DataForgeError) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

/// A lightweight representation of a RowBatch in Python.
#[pyclass]
pub struct PyRowBatch {
    inner: dataforge_core::types::RowBatch,
}

#[pymethods]
impl PyRowBatch {
    #[getter]
    pub fn row_count(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers.clone()
    }

    /// Retrieve a row by its index.
    pub fn get_row(&self, index: usize) -> Option<Vec<PyObject>> {
        Python::with_gil(|py| {
            self.inner.rows.get(index).map(|row| {
                row.cells
                    .iter()
                    .map(|cell| match cell {
                        CellValue::Null => py.None(),
                        CellValue::Bool(b) => b.into_py(py),
                        CellValue::Int(i) => i.into_py(py),
                        CellValue::Float(f) => f.into_py(py),
                        _ => cell.to_display_string().into_py(py),
                    })
                    .collect()
            })
        })
    }

    /// Convert the batch to a list of dicts (ideal for loading into Pandas).
    pub fn to_dicts(&self) -> PyResult<Vec<PyObject>> {
        Python::with_gil(|py| {
            let headers = self.inner.headers.as_ref();
            let mut list = Vec::with_capacity(self.inner.len());

            for row in &self.inner.rows {
                let dict = pyo3::types::PyDict::new_bound(py);
                for (col_idx, cell) in row.cells.iter().enumerate() {
                    let col_name = headers
                        .and_then(|h| h.get(col_idx))
                        .map(|s| s.clone())
                        .unwrap_or_else(|| format!("col_{}", col_idx));

                    let val = match cell {
                        CellValue::Null => py.None(),
                        CellValue::Bool(b) => b.into_py(py),
                        CellValue::Int(i) => i.into_py(py),
                        CellValue::Float(f) => f.into_py(py),
                        _ => cell.to_display_string().into_py(py),
                    };
                    dict.set_item(col_name, val)?;
                }
                list.push(dict.into());
            }

            Ok(list)
        })
    }

    /// Convert the batch to a dictionary of columns (highly optimized for Pandas/Polars).
    pub fn to_column_dict(&self) -> PyResult<PyObject> {
        Python::with_gil(|py| {
            let headers = self.inner.headers.as_ref();
            let num_rows = self.inner.len();
            let num_cols = headers.map(|h| h.len()).unwrap_or(0);

            let dict = pyo3::types::PyDict::new_bound(py);
            if num_rows == 0 {
                return Ok(dict.into());
            }

            // Transpose row-major cells into column-major vectors
            let mut columns = vec![Vec::with_capacity(num_rows); num_cols];
            for row in &self.inner.rows {
                for (col_idx, cell) in row.cells.iter().enumerate() {
                    if col_idx < num_cols {
                        columns[col_idx].push(cell);
                    }
                }
            }

            for col_idx in 0..num_cols {
                let col_name = headers
                    .and_then(|h| h.get(col_idx))
                    .map(|s| s.clone())
                    .unwrap_or_else(|| format!("col_{}", col_idx));

                let col_vec: Vec<PyObject> = columns[col_idx]
                    .iter()
                    .map(|cell| match cell {
                        CellValue::Null => py.None(),
                        CellValue::Bool(b) => b.into_py(py),
                        CellValue::Int(i) => i.into_py(py),
                        CellValue::Float(f) => f.into_py(py),
                        _ => cell.to_display_string().into_py(py),
                    })
                    .collect();

                dict.set_item(col_name, col_vec)?;
            }

            Ok(dict.into())
        })
    }

    /// Convert the batch directly to a Polars DataFrame.
    pub fn to_polars(&self, py: Python<'_>) -> PyResult<PyObject> {
        let col_dict = self.to_column_dict()?;
        let pl = py.import_bound("polars")?;
        let df = pl.call_method1("DataFrame", (col_dict,))?;
        Ok(df.into())
    }

    /// Render batch to a styled HTML report for PDF printing.
    pub fn to_html_report(&self, title: String, dark_mode: bool) -> String {
        let generator = dataforge_core::PdfReportGenerator::new(title).with_dark_mode(dark_mode);
        generator.render_html(&self.inner).unwrap_or_else(|e| format!("Error generating report: {e}"))
    }

/// Streaming CSV reader for Python.
#[pyclass]
pub struct PyCsvReader {
    inner: CsvReader,
}

#[pymethods]
impl PyCsvReader {
    #[new]
    #[pyo3(signature = (path, batch_size=None, parallel=None))]
    pub fn new(path: String, batch_size: Option<usize>, parallel: Option<bool>) -> PyResult<Self> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs);
        }
        if let Some(p) = parallel {
            config = config.with_parallel(p);
        }

        CsvReader::open(&path, config)
            .map(|r| PyCsvReader { inner: r })
            .map_err(to_py_err)
    }

    #[getter]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers().map(|h| h.to_vec())
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<PyRowBatch>> {
        let reader = &mut slf.inner;
        let res = py.allow_threads(|| reader.next_batch());
        match res {
            Some(Ok(batch)) => Ok(Some(PyRowBatch { inner: batch })),
            Some(Err(e)) => Err(to_py_err(e)),
            None => Ok(None),
        }
    }
}

/// Streaming XLSX reader for Python.
#[pyclass]
pub struct PyXlsxReader {
    inner: XlsxReader,
}

#[pymethods]
impl PyXlsxReader {
    #[new]
    #[pyo3(signature = (path, batch_size=None, sheet_name=None))]
    pub fn new(path: String, batch_size: Option<usize>, sheet_name: Option<String>) -> PyResult<Self> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs);
        }
        if let Some(ref sn) = sheet_name {
            config = config.with_sheet_name(sn);
        }

        XlsxReader::open(&path, config)
            .map(|r| PyXlsxReader { inner: r })
            .map_err(to_py_err)
    }

    #[getter]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers().map(|h| h.to_vec())
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<PyRowBatch>> {
        let reader = &mut slf.inner;
        let res = py.allow_threads(|| reader.next_batch());
        match res {
            Some(Ok(batch)) => Ok(Some(PyRowBatch { inner: batch })),
            Some(Err(e)) => Err(to_py_err(e)),
            None => Ok(None),
        }
    }
}

/// Streaming ODS reader for Python.
#[pyclass]
pub struct PyOdsReader {
    inner: OdsReader,
}

#[pymethods]
impl PyOdsReader {
    #[new]
    #[pyo3(signature = (path, batch_size=None, sheet_name=None))]
    pub fn new(path: String, batch_size: Option<usize>, sheet_name: Option<String>) -> PyResult<Self> {
        let mut config = ReaderConfig::default();
        if let Some(bs) = batch_size {
            config = config.with_batch_size(bs);
        }
        if let Some(ref sn) = sheet_name {
            config = config.with_sheet_name(sn);
        }

        OdsReader::open(&path, config)
            .map(|r| PyOdsReader { inner: r })
            .map_err(to_py_err)
    }

    #[getter]
    pub fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers().map(|h| h.to_vec())
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<PyRowBatch>> {
        let reader = &mut slf.inner;
        let res = py.allow_threads(|| reader.next_batch());
        match res {
            Some(Ok(batch)) => Ok(Some(PyRowBatch { inner: batch })),
            Some(Err(e)) => Err(to_py_err(e)),
            None => Ok(None),
        }
    }
}

/// High-performance file format converter: CSV to XLSX.
#[pyfunction]
pub fn convert_csv_to_xlsx(input_path: String, output_path: String, py: Python<'_>) -> PyResult<u64> {
    py.allow_threads(|| {
        dataforge_core::convert::convert_csv_to_xlsx(
            &input_path,
            &output_path,
            ReaderConfig::default(),
            WriterConfig::default(),
        )
        .map_err(to_py_err)
    })
}

/// High-performance file format converter: XLSX to CSV.
#[pyfunction]
pub fn convert_xlsx_to_csv(input_path: String, output_path: String, py: Python<'_>) -> PyResult<u64> {
    py.allow_threads(|| {
        dataforge_core::convert::convert_xlsx_to_csv(
            &input_path,
            &output_path,
            ReaderConfig::default(),
            WriterConfig::default(),
        )
        .map_err(to_py_err)
    })
}

/// The Python module entrypoint.
#[pymodule]
fn dataforge(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyRowBatch>()?;
    m.add_class::<PyCsvReader>()?;
    m.add_class::<PyXlsxReader>()?;
    m.add_class::<PyOdsReader>()?;
    m.add_function(wrap_pyfunction!(convert_csv_to_xlsx, m)?)?;
    m.add_function(wrap_pyfunction!(convert_xlsx_to_csv, m)?)?;
    Ok(())
}

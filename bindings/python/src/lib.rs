use std::path::PathBuf;

use cove_ai_adapters::{
    open as open_archive, write_export_file, AiArchiveOpenOptions, AiExportFormat, AiExportOptions,
    AiSampleIteratorOptions, AiVerifyOptions,
};
use pyo3::{
    exceptions::PyRuntimeError,
    prelude::*,
    types::{PyBytes, PyModule},
    IntoPyObjectExt,
};
use serde_json::Value;

#[pyclass(name = "TrainingArchive")]
struct PyTrainingArchive {
    path: String,
    cove_ai: Option<String>,
    dataset_dir: Option<String>,
}

#[pyclass(name = "TrainingSampleIterator")]
struct PyTrainingSampleIterator {
    archive: cove_ai_adapters::AiTrainingArchive,
    split: Option<String>,
    include_payloads: bool,
    index: usize,
}

#[pymethods]
impl PyTrainingArchive {
    fn verify(&self, py: Python<'_>, policy_report: Option<bool>) -> PyResult<Py<PyAny>> {
        let archive = self.open_native()?;
        let value = archive
            .verify(AiVerifyOptions {
                policy_report: policy_report.unwrap_or(true),
                strict_training: false,
            })
            .map_err(py_error)?;
        json_to_py(py, &value)
    }

    fn training_samples(
        &self,
        py: Python<'_>,
        split: Option<String>,
        include_payloads: Option<bool>,
    ) -> PyResult<Py<PyAny>> {
        let archive = self.open_native()?;
        let rows = archive
            .training_samples(AiSampleIteratorOptions {
                split,
                include_payloads: include_payloads.unwrap_or(false),
            })
            .map_err(py_error)?;
        json_to_py(py, &Value::Array(rows))
    }

    fn training_sample_count(&self, split: Option<String>) -> PyResult<usize> {
        let archive = self.open_native()?;
        archive
            .training_sample_count(split.as_deref())
            .map_err(py_error)
    }

    fn training_sample_at(
        &self,
        py: Python<'_>,
        index: usize,
        split: Option<String>,
        include_payloads: Option<bool>,
    ) -> PyResult<Py<PyAny>> {
        let archive = self.open_native()?;
        let row = archive
            .training_sample_at(
                index,
                AiSampleIteratorOptions {
                    split,
                    include_payloads: include_payloads.unwrap_or(false),
                },
            )
            .map_err(py_error)?;
        json_to_py(py, &row.unwrap_or(Value::Null))
    }

    fn iter_training_samples(
        &self,
        split: Option<String>,
        include_payloads: Option<bool>,
    ) -> PyResult<PyTrainingSampleIterator> {
        Ok(PyTrainingSampleIterator {
            archive: self.open_native()?,
            split,
            include_payloads: include_payloads.unwrap_or(false),
            index: 0,
        })
    }

    fn chunks(&self, py: Python<'_>, include_text: Option<bool>) -> PyResult<Py<PyAny>> {
        let archive = self.open_native()?;
        let rows = archive
            .chunks(include_text.unwrap_or(false))
            .map_err(py_error)?;
        json_to_py(py, &Value::Array(rows))
    }

    fn tokens(&self, py: Python<'_>, include_payloads: Option<bool>) -> PyResult<Py<PyAny>> {
        let archive = self.open_native()?;
        let rows = archive
            .tokens(include_payloads.unwrap_or(false))
            .map_err(py_error)?;
        json_to_py(py, &Value::Array(rows))
    }

    fn multimodal(&self, py: Python<'_>, include_payloads: Option<bool>) -> PyResult<Py<PyAny>> {
        let archive = self.open_native()?;
        let rows = archive
            .multimodal(include_payloads.unwrap_or(false))
            .map_err(py_error)?;
        json_to_py(py, &Value::Array(rows))
    }

    fn export(
        &self,
        py: Python<'_>,
        format: Option<String>,
        out: Option<String>,
        split: Option<String>,
        include_payloads: Option<bool>,
    ) -> PyResult<Py<PyAny>> {
        let archive = self.open_native()?;
        let format = AiExportFormat::parse(format.as_deref().unwrap_or("jsonl")).map_err(py_error)?;
        let options = AiExportOptions {
            format,
            out: out.as_ref().map(PathBuf::from),
            split,
            include_payloads: include_payloads.unwrap_or(false),
            policy_report: true,
        };
        let data = archive.export(options).map_err(py_error)?;
        if let Some(out) = out {
            write_export_file(data.clone(), Some(PathBuf::from(out))).map_err(py_error)?;
        }
        if data.media_type.starts_with("application/json")
            || data.media_type == "application/x-ndjson"
        {
            String::from_utf8_lossy(&data.bytes)
                .to_string()
                .into_py_any(py)
        } else {
            Ok(PyBytes::new(py, &data.bytes).into_any().unbind())
        }
    }
}

#[pymethods]
impl PyTrainingSampleIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let row = self
            .archive
            .training_sample_at(
                self.index,
                AiSampleIteratorOptions {
                    split: self.split.clone(),
                    include_payloads: self.include_payloads,
                },
            )
            .map_err(py_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        self.index += 1;
        Ok(Some(json_to_py(py, &row)?))
    }
}

impl PyTrainingArchive {
    fn open_native(&self) -> PyResult<cove_ai_adapters::AiTrainingArchive> {
        open_archive(
            &self.path,
            AiArchiveOpenOptions {
                cove_ai: self.cove_ai.as_ref().map(PathBuf::from),
                dataset_dir: self.dataset_dir.as_ref().map(PathBuf::from),
            },
        )
        .map_err(py_error)
    }
}

#[pyfunction]
#[pyo3(signature = (path, cove_ai=None, dataset_dir=None))]
fn open(path: String, cove_ai: Option<String>, dataset_dir: Option<String>) -> PyTrainingArchive {
    PyTrainingArchive {
        path,
        cove_ai,
        dataset_dir,
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyTrainingArchive>()?;
    module.add_class::<PyTrainingSampleIterator>()?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    Ok(())
}

fn json_to_py(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    let json = PyModule::import(py, "json")?;
    let text = serde_json::to_string(value).map_err(py_error)?;
    Ok(json.call_method1("loads", (text,))?.unbind())
}

fn py_error(error: impl ToString) -> PyErr {
    PyRuntimeError::new_err(format!("COVE_AI_ERROR: {}", error.to_string()))
}

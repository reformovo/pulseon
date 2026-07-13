use std::sync::Arc;

use duckdb::arrow::array::{
    ArrayRef, Float64Array, Int64Array, StringArray, TimestampMillisecondArray, UInt64Array,
};
use duckdb::arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use duckdb::arrow::error::ArrowError;
use duckdb::arrow::ffi_stream::FFI_ArrowArrayStream;
use duckdb::arrow::record_batch::{RecordBatch, RecordBatchIterator};
use pyo3::prelude::*;
use pyo3::types::PyCapsule;

use crate::model::metric::{MetricAggregate, MetricPoint};

#[pyclass(name = "ArrowTable", module = "pulseon._pulseon")]
pub struct PyArrowTable {
    batch: RecordBatch,
    source_row_count: usize,
    downsampled: bool,
}

impl PyArrowTable {
    pub fn from_metric_points(
        points: &[MetricPoint],
        source_row_count: usize,
        downsampled: bool,
    ) -> Result<Self, ArrowError> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("run_id", DataType::Utf8, false),
            Field::new("metric_key", DataType::Utf8, false),
            Field::new("step", DataType::Int64, false),
            Field::new(
                "timestamp",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                false,
            ),
            Field::new("value_f64", DataType::Float64, false),
            Field::new(
                "ingested_at",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                false,
            ),
        ]));
        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from_iter_values(
                points.iter().map(|point| point.run_id.as_str()),
            )),
            Arc::new(StringArray::from_iter_values(
                points.iter().map(|point| point.metric_key.as_str()),
            )),
            Arc::new(Int64Array::from_iter_values(
                points.iter().map(|point| point.step.value()),
            )),
            Arc::new(
                TimestampMillisecondArray::from_iter_values(
                    points
                        .iter()
                        .map(|point| point.timestamp.timestamp_millis()),
                )
                .with_timezone("UTC"),
            ),
            Arc::new(Float64Array::from_iter_values(
                points.iter().map(|point| point.value_f64),
            )),
            Arc::new(
                TimestampMillisecondArray::from_iter_values(
                    points
                        .iter()
                        .map(|point| point.ingested_at.timestamp_millis()),
                )
                .with_timezone("UTC"),
            ),
        ];
        Ok(Self {
            batch: RecordBatch::try_new(schema, columns)?,
            source_row_count,
            downsampled,
        })
    }

    pub fn from_metric_summaries(summaries: &[MetricAggregate]) -> Result<Self, ArrowError> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("run_id", DataType::Utf8, false),
            Field::new("metric_key", DataType::Utf8, false),
            Field::new("effective_count", DataType::UInt64, false),
            Field::new("last_step", DataType::Int64, false),
            Field::new("last_value_f64", DataType::Float64, false),
            Field::new("min_value_f64", DataType::Float64, false),
            Field::new("max_value_f64", DataType::Float64, false),
        ]));
        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from_iter_values(
                summaries.iter().map(|summary| summary.run_id.as_str()),
            )),
            Arc::new(StringArray::from_iter_values(
                summaries.iter().map(|summary| summary.metric_key.as_str()),
            )),
            Arc::new(UInt64Array::from_iter_values(
                summaries.iter().map(|summary| summary.effective_count),
            )),
            Arc::new(Int64Array::from_iter_values(
                summaries.iter().map(|summary| summary.last_step.value()),
            )),
            Arc::new(Float64Array::from_iter_values(
                summaries.iter().map(|summary| summary.last_value_f64),
            )),
            Arc::new(Float64Array::from_iter_values(
                summaries.iter().map(|summary| summary.min_value_f64),
            )),
            Arc::new(Float64Array::from_iter_values(
                summaries.iter().map(|summary| summary.max_value_f64),
            )),
        ];
        Ok(Self {
            batch: RecordBatch::try_new(schema, columns)?,
            source_row_count: summaries.len(),
            downsampled: false,
        })
    }
}

#[pymethods]
impl PyArrowTable {
    #[getter]
    fn row_count(&self) -> usize {
        self.batch.num_rows()
    }

    #[getter]
    const fn source_row_count(&self) -> usize {
        self.source_row_count
    }

    #[getter]
    const fn downsampled(&self) -> bool {
        self.downsampled
    }

    #[getter]
    fn column_names(&self) -> Vec<&str> {
        self.batch
            .schema_ref()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect()
    }

    #[pyo3(signature = (_requested_schema=None))]
    fn __arrow_c_stream__<'py>(
        &self,
        py: Python<'py>,
        _requested_schema: Option<&Bound<'py, PyCapsule>>,
    ) -> PyResult<Bound<'py, PyCapsule>> {
        let schema = self.batch.schema();
        let batches = vec![Ok::<_, ArrowError>(self.batch.clone())];
        let reader = RecordBatchIterator::new(batches.into_iter(), schema);
        PyCapsule::new_with_value(
            py,
            FFI_ArrowArrayStream::new(Box::new(reader)),
            c"arrow_array_stream",
        )
    }
}

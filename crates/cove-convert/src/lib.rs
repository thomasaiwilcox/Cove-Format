//! Conversion facade for stable COVE v2 import APIs.

pub mod cli;
pub mod source;

pub use cove_arrow::convert;
pub use source::{
    convert_bytes_to_cove, convert_file_to_cove, detect_source_format, read_arrow_batches,
    read_arrow_batches_from_bytes, read_csv_batches, read_orc_batches, read_orc_batches_from_bytes,
    read_parquet_batches, read_parquet_batches_from_bytes, schema_fingerprint, source_digest,
    ConversionOptions, ConvertError, CsvReadOptions, SourceFormat,
};

pub use cove_arrow::convert::ParquetConversionResult as ConversionResult;

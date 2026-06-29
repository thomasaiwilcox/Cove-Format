use arrow_array::RecordBatch;
use cove_core::CoveError;

use super::stats::{DecodeStats, DecodedScan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeControl {
    Continue,
    Stop,
}

pub(crate) trait DecodeSink {
    fn emit_batch(
        &mut self,
        batch: RecordBatch,
        stats: &mut DecodeStats,
    ) -> Result<DecodeControl, CoveError>;

    fn should_stop(&self) -> bool {
        false
    }
}

#[derive(Debug, Default)]
pub(crate) struct VecDecodeSink {
    pub(crate) batches: Vec<RecordBatch>,
}

impl VecDecodeSink {
    pub(crate) fn finish(self, stats: DecodeStats) -> DecodedScan {
        DecodedScan {
            batches: self.batches,
            stats,
        }
    }
}

impl DecodeSink for VecDecodeSink {
    fn emit_batch(
        &mut self,
        batch: RecordBatch,
        stats: &mut DecodeStats,
    ) -> Result<DecodeControl, CoveError> {
        stats.rows_materialized += batch.num_rows();
        self.batches.push(batch);
        Ok(DecodeControl::Continue)
    }
}

#[derive(Debug)]
pub(crate) struct FetchLimitedDecodeSink<S> {
    pub(crate) inner: S,
    remaining: Option<usize>,
    stopped: bool,
}

impl<S> FetchLimitedDecodeSink<S> {
    pub(crate) fn new(inner: S, fetch: Option<usize>) -> Self {
        Self {
            inner,
            remaining: fetch,
            stopped: fetch == Some(0),
        }
    }
}

impl<S: DecodeSink> DecodeSink for FetchLimitedDecodeSink<S> {
    fn emit_batch(
        &mut self,
        batch: RecordBatch,
        stats: &mut DecodeStats,
    ) -> Result<DecodeControl, CoveError> {
        if self.stopped {
            return Ok(DecodeControl::Stop);
        }

        let batch = match self.remaining {
            Some(0) => {
                self.stopped = true;
                return Ok(DecodeControl::Stop);
            }
            Some(remaining) if batch.num_rows() > remaining => batch.slice(0, remaining),
            _ => batch,
        };
        let emitted_rows = batch.num_rows();
        let control = self.inner.emit_batch(batch, stats)?;
        if let Some(remaining) = self.remaining.as_mut() {
            *remaining = remaining.saturating_sub(emitted_rows);
            if *remaining == 0 {
                self.stopped = true;
                return Ok(DecodeControl::Stop);
            }
        }
        if control == DecodeControl::Stop {
            self.stopped = true;
        }
        Ok(control)
    }

    fn should_stop(&self) -> bool {
        self.stopped || self.inner.should_stop()
    }
}

pub(crate) fn emit_batch<S: DecodeSink + ?Sized>(
    sink: &mut S,
    stats: &mut DecodeStats,
    batch: RecordBatch,
) -> Result<DecodeControl, CoveError> {
    sink.emit_batch(batch, stats)
}

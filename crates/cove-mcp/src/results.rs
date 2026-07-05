use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};

use crate::config::CoveMcpError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResultHandle(String);

impl ResultHandle {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct PagedResult {
    pub values: Vec<Value>,
    pub page_size: usize,
    pub max_response_bytes: usize,
    pub created_at: Instant,
    pub ttl: Duration,
    pub metadata: Value,
}

impl PagedResult {
    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.created_at) > self.ttl
    }
}

#[derive(Debug)]
pub struct ResultStore {
    inner: Mutex<ResultStoreInner>,
    ttl: Duration,
    max_handles: usize,
}

#[derive(Debug, Default)]
struct ResultStoreInner {
    sequence: u64,
    handles: BTreeMap<String, PagedResult>,
    order: VecDeque<String>,
}

impl ResultStore {
    pub fn new(ttl: Duration, max_handles: usize) -> Self {
        Self {
            inner: Mutex::new(ResultStoreInner::default()),
            ttl,
            max_handles,
        }
    }

    pub fn insert(
        &self,
        values: Vec<Value>,
        page_size: usize,
        max_response_bytes: usize,
        metadata: Value,
    ) -> ResultHandle {
        let mut inner = self.inner.lock().expect("result store lock poisoned");
        prune_locked(&mut inner, self.max_handles, Instant::now());
        inner.sequence = inner.sequence.saturating_add(1);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let handle = format!("result-{millis}-{}", inner.sequence);
        inner.order.push_back(handle.clone());
        inner.handles.insert(
            handle.clone(),
            PagedResult {
                values,
                page_size,
                max_response_bytes,
                created_at: Instant::now(),
                ttl: self.ttl,
                metadata,
            },
        );
        ResultHandle(handle)
    }

    pub fn page(&self, handle: &str, offset: usize) -> Result<Value, CoveMcpError> {
        let mut inner = self.inner.lock().expect("result store lock poisoned");
        prune_locked(&mut inner, self.max_handles, Instant::now());
        let result = inner.handles.get(handle).ok_or_else(|| {
            CoveMcpError::Query(format!("unknown or expired result handle `{handle}`"))
        })?;
        Ok(page_value(handle, result, offset))
    }
}

fn prune_locked(inner: &mut ResultStoreInner, max_handles: usize, now: Instant) {
    inner.handles.retain(|_, result| !result.expired(now));
    inner
        .order
        .retain(|handle| inner.handles.contains_key(handle));
    while inner.handles.len() > max_handles {
        let Some(oldest) = inner.order.pop_front() else {
            break;
        };
        inner.handles.remove(&oldest);
    }
}

pub fn page_value(handle: &str, result: &PagedResult, offset: usize) -> Value {
    let mut rows = Vec::new();
    let mut bytes = 2usize;
    for value in result.values.iter().skip(offset).take(result.page_size) {
        let next_bytes =
            serde_json::to_vec(value).map_or(result.max_response_bytes + 1, |v| v.len());
        if !rows.is_empty() && bytes.saturating_add(next_bytes) > result.max_response_bytes {
            break;
        }
        bytes = bytes.saturating_add(next_bytes).saturating_add(1);
        rows.push(value.clone());
    }
    let next_offset = offset.saturating_add(rows.len());
    let has_more = next_offset < result.values.len();
    json!({
        "result_handle": handle,
        "offset": offset,
        "next_offset": if has_more { Some(next_offset) } else { None },
        "row_count": result.values.len(),
        "rows": rows,
        "metadata": result.metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_results() {
        let store = ResultStore::new(Duration::from_secs(30), 8);
        let handle = store.insert(vec![json!({"a": 1}), json!({"a": 2})], 1, 1024, json!({}));
        let page = store.page(handle.as_str(), 0).unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(), 1);
        assert_eq!(page["next_offset"], json!(1));
    }
}

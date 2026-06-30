use cove_core::profile::cove_map::MapProjectionEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RequestedProjectionId<'a>(&'a str);

impl<'a> RequestedProjectionId<'a> {
    pub(super) fn new(value: &'a str) -> Self {
        Self(value)
    }

    pub(super) fn as_str(self) -> &'a str {
        self.0
    }

    pub(super) fn matches_entry(self, projection: &MapProjectionEntry) -> bool {
        projection.projection_id == self.0
    }
}

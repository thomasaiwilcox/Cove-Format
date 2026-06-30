#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct EffectiveSourceRef(u32);

impl EffectiveSourceRef {
    pub(super) fn from_raw(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct VectorSpaceId(u32);

impl VectorSpaceId {
    pub(super) fn from_raw(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ModelInputDigestRef(u32);

impl ModelInputDigestRef {
    pub(super) fn non_zero(value: u32) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub(super) fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ModelInputVectorKey {
    pub(super) effective_source: EffectiveSourceRef,
    pub(super) vector_space_id: VectorSpaceId,
    pub(super) model_input_digest_ref: ModelInputDigestRef,
}

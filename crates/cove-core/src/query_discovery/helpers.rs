use crate::{
    constants::CoveLogicalType,
    profile::cove_o::{
        ObjectTypeEntryV1, OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_EVENT_OBJECT,
        OBJECT_TYPE_FLAG_EVIDENCE_OBJECT, OBJECT_TYPE_FLAG_LINK_OBJECT,
        OBJECT_TYPE_FLAG_PROJECTION_OBJECT,
    },
};

use super::MetadataDisclosureMode;

pub(super) fn rounded_count_hint(row_count: u64) -> u64 {
    if row_count <= 1000 {
        row_count
    } else {
        let bucket = if row_count < 10_000 {
            100
        } else if row_count < 1_000_000 {
            1_000
        } else {
            100_000
        };
        (row_count / bucket) * bucket
    }
}

pub(super) fn object_kind(object_type: &ObjectTypeEntryV1) -> &'static str {
    if object_type.flags & (OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT | OBJECT_TYPE_FLAG_LINK_OBJECT) != 0
        || object_type.type_name.starts_with("Association:")
    {
        "association"
    } else if object_type.flags & OBJECT_TYPE_FLAG_EVIDENCE_OBJECT != 0 {
        "evidence"
    } else if object_type.flags & OBJECT_TYPE_FLAG_PROJECTION_OBJECT != 0 {
        "projection"
    } else if object_type.flags & OBJECT_TYPE_FLAG_EVENT_OBJECT != 0 {
        "event"
    } else {
        "entity"
    }
}

pub(super) fn logical_type_name(logical: CoveLogicalType) -> &'static str {
    match logical {
        CoveLogicalType::Null => "null",
        CoveLogicalType::Bool => "bool",
        CoveLogicalType::Int8 => "int8",
        CoveLogicalType::Int16 => "int16",
        CoveLogicalType::Int32 => "int32",
        CoveLogicalType::Int64 => "int64",
        CoveLogicalType::UInt8 => "uint8",
        CoveLogicalType::UInt16 => "uint16",
        CoveLogicalType::UInt32 => "uint32",
        CoveLogicalType::UInt64 => "uint64",
        CoveLogicalType::Float32 => "float32",
        CoveLogicalType::Float64 => "float64",
        CoveLogicalType::Decimal64 => "decimal64",
        CoveLogicalType::Decimal128 => "decimal128",
        CoveLogicalType::DateDays => "date_days",
        CoveLogicalType::TimestampMicros => "timestamp_micros",
        CoveLogicalType::TimestampNanos => "timestamp_nanos",
        CoveLogicalType::Utf8 => "utf8",
        CoveLogicalType::Binary => "binary",
        CoveLogicalType::Uuid => "uuid",
        CoveLogicalType::Json => "json",
        CoveLogicalType::List => "list",
        CoveLogicalType::Struct => "struct",
        CoveLogicalType::Map => "map",
    }
}

pub(super) fn column_operations(logical: CoveLogicalType) -> Vec<&'static str> {
    match logical {
        CoveLogicalType::Binary
        | CoveLogicalType::Json
        | CoveLogicalType::List
        | CoveLogicalType::Struct
        | CoveLogicalType::Map => vec!["select"],
        _ => vec!["select", "filter", "order", "group"],
    }
}

pub(super) fn disclosure_label(mode: MetadataDisclosureMode) -> &'static str {
    match mode {
        MetadataDisclosureMode::Public => "public",
        MetadataDisclosureMode::Developer => "developer",
    }
}

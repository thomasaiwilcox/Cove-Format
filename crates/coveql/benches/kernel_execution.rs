use std::{
    collections::{BTreeMap, BTreeSet},
    hint::black_box,
    time::Instant,
};

use cove_core::{
    checksum,
    constants::{
        CompressionCodec, CoveEncodingKind, CoveLogicalType, CovePhysicalKind, PrimaryProfile,
        SectionKind, FEATURE_OBJECT_PROFILE, FEATURE_SEMANTIC_MAP,
    },
    encoding::{
        bit_packed::BitPackedPayload,
        local_codebook::{LocalCodebookPayload, LocalCodebookValues, LocalIndexPayload},
    },
    native::{
        anti_join_i64_eq_left_nulls_unmatched, anti_join_u32_eq, anti_join_u64_eq,
        compact_selection_bitmap, distinct_u32, filter_bool_eq, filter_fixed_bytes_eq,
        filter_fixed_bytes_in, filter_i64_le_range, filter_i64_range,
        filter_length_prefixed_varbytes_eq, filter_length_prefixed_varbytes_in,
        filter_local_u16_membership, filter_local_u32_membership, filter_local_u8_membership,
        filter_numcode_le_in_typed, filter_numcode_le_not_in_typed, filter_numcode_le_typed,
        filter_u32_in_sorted, filter_u32_le_in_sorted, filter_u32_le_not_in_sorted,
        filter_u32_not_in_sorted, filter_u64_eq, filter_u64_le_eq, filter_u64_le_eq_dispatch,
        filter_varbytes_eq, filter_varbytes_in, filter_varbytes_prefix, group_count_u32_hash,
        inner_join_i64_eq, inner_join_u32_eq, inner_join_u64_eq, local_membership_u8,
        native_object_temporal_batch_from_retained_segment, semi_join_i64_eq, semi_join_u64_eq,
        sort_rows_i64_with_stats, top_n_rows_i64_with_stats, valid_rows_except, BoundInclusive,
        LaneRef, NativeCodeDomain, NativeKernelDispatch, NativeNullOrder, NativeNumericLiteral,
        NativeNumericPredicateOp, NativeSortDirection, SelectionBitmap, ValidityRef,
    },
    page::ColumnPageIndexEntryV1,
    page_payload::{ColumnPagePayloadV1, RetainedColumnPagePayloadV1},
    profile::{
        cove_map::{MapEvidenceEntry, MapEvidenceIndex},
        cove_o::{
            ObjectTypeCatalog, ObjectTypeEntryV1, PropertyEntryV1, RecordKind,
            RetainedTemporalPropertyColumn, RetainedTemporalPropertyPage,
            RetainedTemporalSegmentData, TemporalRowEntryV1, TemporalSegmentHeaderV1,
            OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT, OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            PROPERTY_FLAG_ASSOCIATION_FROM_GOID, PROPERTY_FLAG_ASSOCIATION_TO_GOID,
        },
    },
    reader::ValidationOptions,
    retained_bytes::RetainedBytes,
    segment::TableColumnDirectoryEntryV1,
    writer::{MinimalCoveWriter, SectionPayload},
};
use coveql::{
    parse_resolve_and_plan_query, parse_resolve_plan_build_physical_and_execute_query,
    AssociationOptimizationReport, CoveQlOutputMode, EvidenceOptimizationReport, ExecutionOptions,
    KernelExecutionMode, KernelExecutionOptions, MaterializedAssociationRow, OutputGrain,
    ParseOptions, PhysicalPlanOptions, PlanOptions, ResolveOptions, SecurityContext,
};
use serde_json::{json, Value};

fn main() {
    benchmark_native_core_kernels();
    benchmark_object_kernel_execution_report();
    benchmark_association_report();
    benchmark_evidence_report();
}

fn benchmark_native_core_kernels() {
    let rows = 1usize << 20;
    let mut nulls = vec![0u8; rows.div_ceil(8)];
    for row in (0..rows).step_by(17) {
        nulls[row / 8] |= 1u8 << (row % 8);
    }
    let validity = ValidityRef::CoveNullBitmap {
        bytes: &nulls,
        row_count: rows,
    };
    let mut base = SelectionBitmap::all(rows);
    for row in (0..rows).step_by(7) {
        base.clear(row);
    }

    let u64_values = (0..rows).map(|row| (row & 1023) as u64).collect::<Vec<_>>();
    let started = Instant::now();
    let (selected, stats) = filter_u64_eq(&u64_values, validity, 7, Some(&base));
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u64_eq rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let mut u64_le_bytes = Vec::with_capacity(rows * std::mem::size_of::<u64>());
    for value in &u64_values {
        u64_le_bytes.extend_from_slice(&value.to_le_bytes());
    }
    let started = Instant::now();
    let (selected, stats) =
        filter_u64_le_eq(&u64_le_bytes, rows, validity, 7, Some(&base)).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u64_le_eq rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
	    );
    black_box(&selected);

    let all_valid = ValidityRef::AllValid { row_count: rows };
    let started = Instant::now();
    let (auto_selected, auto_stats) = filter_u64_le_eq_dispatch(
        &u64_le_bytes,
        rows,
        all_valid,
        7,
        None,
        NativeKernelDispatch::Auto,
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u64_le_eq_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        auto_stats.rows_seen,
        elapsed_us,
        per_us(auto_stats.rows_seen, elapsed_us),
        auto_stats.rows_matched,
        auto_stats.rows_valid,
        auto_stats.bitmap_words_touched,
        auto_stats.bytes_touched_estimate,
        auto_stats.dispatch
    );
    black_box(&auto_selected);

    let started = Instant::now();
    let (vector, stats) = compact_selection_bitmap(&selected, NativeKernelDispatch::Auto).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=selection_compaction rows={} selected={} elapsed_us={} rows_per_us={:.3} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        vector.len(),
        elapsed_us,
        per_us(vector.len(), elapsed_us),
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&vector);

    let dense_selection = SelectionBitmap::all(rows);
    let started = Instant::now();
    let (vector, stats) =
        compact_selection_bitmap(&dense_selection, NativeKernelDispatch::Auto).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=selection_compaction_dense_auto rows={} selected={} elapsed_us={} rows_per_us={:.3} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        vector.len(),
        elapsed_us,
        per_us(vector.len(), elapsed_us),
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&vector);

    let mut intersect_left = SelectionBitmap::all(rows);
    for row in (0..rows).step_by(5) {
        intersect_left.clear(row);
    }
    let started = Instant::now();
    let dispatch = intersect_left.intersect_with_dispatch(&base, NativeKernelDispatch::Auto);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=bitmap_intersect_auto rows={} elapsed_us={} rows_per_us={:.3} selected={} dispatch={:?}",
        rows,
        elapsed_us,
        per_us(rows, elapsed_us),
        intersect_left.count_ones(),
        dispatch
    );
    black_box(&intersect_left);

    let i64_values = (0..rows)
        .map(|row| (row as i64 & 2047) - 1024)
        .collect::<Vec<_>>();
    let started = Instant::now();
    let (selected, stats) = filter_i64_range(
        &i64_values,
        validity,
        Some((-10, BoundInclusive::Inclusive)),
        Some((10, BoundInclusive::Inclusive)),
        Some(&base),
    );
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_range rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let mut i64_le_bytes = Vec::with_capacity(rows * std::mem::size_of::<i64>());
    for value in &i64_values {
        i64_le_bytes.extend_from_slice(&value.to_le_bytes());
    }
    let started = Instant::now();
    let (selected, stats) = filter_i64_le_range(
        &i64_le_bytes,
        rows,
        validity,
        Some((-10, BoundInclusive::Inclusive)),
        Some((10, BoundInclusive::Inclusive)),
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_le_range rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_i64_le_range(
        &i64_le_bytes,
        rows,
        all_valid,
        Some((-10, BoundInclusive::Inclusive)),
        Some((10, BoundInclusive::Inclusive)),
        None,
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_le_range_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let started = Instant::now();
    let (sorted_rows, stats) = sort_rows_i64_with_stats(
        &i64_values,
        validity,
        NativeSortDirection::Descending,
        NativeNullOrder::Last,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_row_id_sort rows={} elapsed_us={} rows_per_us={:.3} selected={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        sorted_rows.len(),
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&sorted_rows);

    let started = Instant::now();
    let (top_rows, stats) = top_n_rows_i64_with_stats(
        &i64_values,
        validity,
        NativeSortDirection::Descending,
        NativeNullOrder::Last,
        Some(&base),
        128,
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_top_n rows={} limit=128 elapsed_us={} rows_per_us={:.3} selected={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        top_rows.len(),
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&top_rows);

    let started = Instant::now();
    let (selected, stats) = filter_numcode_le_typed(
        &i64_le_bytes,
        rows,
        validity,
        CoveLogicalType::Int64,
        NativeNumericPredicateOp::GtEq,
        NativeNumericLiteral::Int64(-10),
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=numcode_i64_le_gte rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_numcode_le_typed(
        &i64_le_bytes,
        rows,
        all_valid,
        CoveLogicalType::Int64,
        NativeNumericPredicateOp::GtEq,
        NativeNumericLiteral::Int64(-10),
        None,
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=numcode_i64_le_gte_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let single_numeric_literal = [NativeNumericLiteral::Int64(-10)];
    let started = Instant::now();
    let (selected, stats) = filter_numcode_le_in_typed(
        &i64_le_bytes,
        rows,
        all_valid,
        CoveLogicalType::Int64,
        &single_numeric_literal,
        None,
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=numcode_i64_le_single_in_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_numcode_le_not_in_typed(
        &i64_le_bytes,
        rows,
        all_valid,
        CoveLogicalType::Int64,
        &single_numeric_literal,
        None,
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=numcode_i64_le_single_not_in_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let numeric_literals = [
        NativeNumericLiteral::Int64(-10),
        NativeNumericLiteral::Int64(0),
        NativeNumericLiteral::Int64(10),
    ];
    let started = Instant::now();
    let (selected, stats) = filter_numcode_le_in_typed(
        &i64_le_bytes,
        rows,
        validity,
        CoveLogicalType::Int64,
        &numeric_literals,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=numcode_i64_le_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_numcode_le_not_in_typed(
        &i64_le_bytes,
        rows,
        validity,
        CoveLogicalType::Int64,
        &numeric_literals,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=numcode_i64_le_not_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let u32_values = (0..rows).map(|row| (row & 4095) as u32).collect::<Vec<_>>();
    let filecode_needles = [3u32, 17, 99, 255, 1024, 4095];
    let started = Instant::now();
    let (selected, stats) =
        filter_u32_in_sorted(&u32_values, validity, &filecode_needles, Some(&base));
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_u32_in_sorted(&u32_values, all_valid, &filecode_needles, None);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_in_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) =
        filter_u32_not_in_sorted(&u32_values, validity, &filecode_needles, Some(&base));
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_not_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) =
        filter_u32_not_in_sorted(&u32_values, all_valid, &filecode_needles, None);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_not_in_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let mut u32_le_bytes = Vec::with_capacity(rows * std::mem::size_of::<u32>());
    for value in &u32_values {
        u32_le_bytes.extend_from_slice(&value.to_le_bytes());
    }
    let started = Instant::now();
    let (selected, stats) = filter_u32_le_in_sorted(
        &u32_le_bytes,
        rows,
        validity,
        &filecode_needles,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_le_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_u32_le_not_in_sorted(
        &u32_le_bytes,
        rows,
        validity,
        &filecode_needles,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_le_not_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) =
        filter_u32_le_not_in_sorted(&u32_le_bytes, rows, all_valid, &[7], None).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_le_not_in_single_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let started = Instant::now();
    let (groups, stats) = group_count_u32_hash(&u32_values, validity, Some(&base)).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_hash_group rows={} elapsed_us={} rows_per_us={:.3} groups={} grouped={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        groups.counts.len(),
        groups.rows_grouped,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&groups);

    let started = Instant::now();
    let (distinct, stats) = distinct_u32(&u32_values, validity, Some(&base)).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_distinct rows={} elapsed_us={} rows_per_us={:.3} distinct={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        distinct.len(),
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&distinct);

    let join_domain = NativeCodeDomain {
        semantic_domain_id: Some("bench-u32-domain".into()),
        dictionary_epoch: Some(1),
        ..NativeCodeDomain::default()
    };
    let right_values = (0..1024u32).step_by(3).collect::<Vec<_>>();
    let started = Instant::now();
    let (pairs, stats) = inner_join_u32_eq(
        &u32_values,
        validity,
        &join_domain,
        &right_values,
        ValidityRef::AllValid {
            row_count: right_values.len(),
        },
        &join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_inner_join rows={} elapsed_us={} rows_per_us={:.3} pairs={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&pairs);

    let started = Instant::now();
    let (selected, stats) = anti_join_u32_eq(
        &u32_values,
        validity,
        &join_domain,
        &right_values,
        ValidityRef::AllValid {
            row_count: right_values.len(),
        },
        &join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u32_anti_join rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let u64_join_domain = NativeCodeDomain {
        dictionary_id: Some("bench-u64-domain".into()),
        dictionary_epoch: Some(1),
        ..NativeCodeDomain::default()
    };
    let right_u64_values = (0..1024u64).step_by(5).collect::<Vec<_>>();
    let started = Instant::now();
    let (pairs, stats) = inner_join_u64_eq(
        &u64_values,
        validity,
        &u64_join_domain,
        &right_u64_values,
        ValidityRef::AllValid {
            row_count: right_u64_values.len(),
        },
        &u64_join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u64_inner_join rows={} elapsed_us={} rows_per_us={:.3} pairs={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&pairs);

    let started = Instant::now();
    let (selected, stats) = semi_join_u64_eq(
        &u64_values,
        validity,
        &u64_join_domain,
        &right_u64_values,
        ValidityRef::AllValid {
            row_count: right_u64_values.len(),
        },
        &u64_join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u64_semi_join rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = anti_join_u64_eq(
        &u64_values,
        validity,
        &u64_join_domain,
        &right_u64_values,
        ValidityRef::AllValid {
            row_count: right_u64_values.len(),
        },
        &u64_join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=u64_anti_join rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let i64_join_domain = NativeCodeDomain {
        semantic_domain_id: Some("bench-i64-domain".into()),
        null_policy: Some("validity-bitmap-nulls-never-match".into()),
        ..NativeCodeDomain::default()
    };
    let right_i64_values = (-1024..1024i64).step_by(11).collect::<Vec<_>>();
    let started = Instant::now();
    let (pairs, stats) = inner_join_i64_eq(
        &i64_values,
        validity,
        &i64_join_domain,
        &right_i64_values,
        ValidityRef::AllValid {
            row_count: right_i64_values.len(),
        },
        &i64_join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_inner_join rows={} elapsed_us={} rows_per_us={:.3} pairs={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&pairs);

    let started = Instant::now();
    let (selected, stats) = semi_join_i64_eq(
        &i64_values,
        validity,
        &i64_join_domain,
        &right_i64_values,
        ValidityRef::AllValid {
            row_count: right_i64_values.len(),
        },
        &i64_join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_semi_join rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = anti_join_i64_eq_left_nulls_unmatched(
        &i64_values,
        validity,
        &i64_join_domain,
        &right_i64_values,
        ValidityRef::AllValid {
            row_count: right_i64_values.len(),
        },
        &i64_join_domain,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=i64_anti_join_left_nulls_unmatched rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let local_to_global = (0..64u64).map(|value| value * 10).collect::<Vec<_>>();
    let global_needles = [20u64, 70, 130, 310, 610];
    let local_membership = local_membership_u8(&local_to_global, &global_needles);
    let local_values = (0..rows).map(|row| (row & 63) as u8).collect::<Vec<_>>();
    let started = Instant::now();
    let (selected, stats) =
        filter_local_u8_membership(&local_values, validity, &local_membership, Some(&base));
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=local_u8_membership rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) =
        filter_local_u8_membership(&local_values, all_valid, &local_membership, None);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=local_u8_membership_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let local_values_u16 = local_values
        .iter()
        .copied()
        .map(u16::from)
        .collect::<Vec<_>>();
    let started = Instant::now();
    let (selected, stats) =
        filter_local_u16_membership(&local_values_u16, all_valid, &local_membership, None);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=local_u16_membership_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let local_values_u32 = local_values
        .iter()
        .copied()
        .map(u32::from)
        .collect::<Vec<_>>();
    let started = Instant::now();
    let (selected, stats) =
        filter_local_u32_membership(&local_values_u32, all_valid, &local_membership, None);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=local_u32_membership_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let bool_values = (0..rows)
        .map(|row| if row & 1 == 0 { 1u8 } else { 0u8 })
        .collect::<Vec<_>>();
    let started = Instant::now();
    let (selected, stats) =
        filter_bool_eq(&bool_values, rows, validity, true, Some(&base)).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=bool_eq rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_bool_eq(&bool_values, rows, all_valid, true, None).unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=bool_eq_auto rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={} dispatch={:?}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate,
        stats.dispatch
    );
    black_box(&selected);

    let mut fixed_values = Vec::with_capacity(rows * 16);
    for row in 0..rows {
        let mut value = [0u8; 16];
        value[0..8].copy_from_slice(&((row & 1023) as u64).to_le_bytes());
        fixed_values.extend_from_slice(&value);
    }
    let mut fixed_needle = [0u8; 16];
    fixed_needle[0..8].copy_from_slice(&7u64.to_le_bytes());
    let started = Instant::now();
    let (selected, stats) = filter_fixed_bytes_eq(
        &fixed_values,
        rows,
        16,
        validity,
        &fixed_needle,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=fixed16_eq rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let mut fixed_needles = Vec::with_capacity(16 * 4);
    for needle in [7u64, 17, 99, 255] {
        let mut value = [0u8; 16];
        value[0..8].copy_from_slice(&needle.to_le_bytes());
        fixed_needles.extend_from_slice(&value);
    }
    let started = Instant::now();
    let (selected, stats) = filter_fixed_bytes_in(
        &fixed_values,
        rows,
        16,
        validity,
        &fixed_needles,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=fixed16_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (matched, stats) =
        filter_fixed_bytes_in(&fixed_values, rows, 16, validity, &fixed_needles, None).unwrap();
    let selected = valid_rows_except(rows, validity, &matched);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=fixed16_not_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        selected.count_ones(),
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let mut varbytes_offsets = Vec::with_capacity(rows);
    let mut varbytes_values = Vec::with_capacity(rows * 12);
    for row in 0..rows {
        varbytes_offsets.push(varbytes_values.len() as u32);
        let value = format!("v{}", row & 1023);
        varbytes_values.extend_from_slice(&(value.len() as u32).to_le_bytes());
        varbytes_values.extend_from_slice(value.as_bytes());
    }
    let started = Instant::now();
    let (selected, stats) = filter_varbytes_eq(
        &varbytes_offsets,
        &varbytes_values,
        validity,
        b"v7",
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=varbytes_eq rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let varbytes_needles = [b"v7".as_slice(), b"v17".as_slice(), b"v99".as_slice()];
    let started = Instant::now();
    let (selected, stats) = filter_varbytes_in(
        &varbytes_offsets,
        &varbytes_values,
        validity,
        &varbytes_needles,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=varbytes_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (matched, stats) = filter_varbytes_in(
        &varbytes_offsets,
        &varbytes_values,
        validity,
        &varbytes_needles,
        None,
    )
    .unwrap();
    let selected = valid_rows_except(rows, validity, &matched);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=varbytes_not_in rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        selected.count_ones(),
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) =
        filter_length_prefixed_varbytes_eq(&varbytes_values, rows, validity, b"v7", Some(&base))
            .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=varbytes_eq_no_offsets rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_length_prefixed_varbytes_in(
        &varbytes_values,
        rows,
        validity,
        &varbytes_needles,
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=varbytes_in_no_offsets rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (matched, stats) = filter_length_prefixed_varbytes_in(
        &varbytes_values,
        rows,
        validity,
        &varbytes_needles,
        None,
    )
    .unwrap();
    let selected = valid_rows_except(rows, validity, &matched);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=varbytes_not_in_no_offsets rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        selected.count_ones(),
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let started = Instant::now();
    let (selected, stats) = filter_varbytes_prefix(
        &varbytes_offsets,
        &varbytes_values,
        validity,
        b"v7",
        Some(&base),
    )
    .unwrap();
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=varbytes_prefix rows={} elapsed_us={} rows_per_us={:.3} matched={} valid={} bitmap_words={} bytes_touched={}",
        stats.rows_seen,
        elapsed_us,
        per_us(stats.rows_seen, elapsed_us),
        stats.rows_matched,
        stats.rows_valid,
        stats.bitmap_words_touched,
        stats.bytes_touched_estimate
    );
    black_box(&selected);

    let retained = synthetic_retained_object_segment(rows.min(1 << 16));
    let started = Instant::now();
    let mut native_pages = 0usize;
    let mut decode_boundaries = 0usize;
    let mut native_rows = 0usize;
    let batch =
        native_object_temporal_batch_from_retained_segment(&retained, NativeCodeDomain::default())
            .expect("retained segment should expose native object temporal batch");
    for page in &batch.property_pages {
        match &page.lane {
            LaneRef::DecodeBoundary { .. } => decode_boundaries += 1,
            _ => {
                native_pages += 1;
                native_rows += page.row_count;
            }
        }
    }
    black_box(&batch);
    let elapsed_us = started.elapsed().as_micros();
    println!(
        "kernel_execution native_core case=cove_o_retained_native_batch segments=1 property_pages={} native_pages={} decode_boundaries={} native_rows={} elapsed_us={} pages_per_us={:.3}",
        native_pages + decode_boundaries,
        native_pages,
        decode_boundaries,
        native_rows,
        elapsed_us,
        per_us(native_pages + decode_boundaries, elapsed_us)
    );
}

fn synthetic_retained_object_segment(rows: usize) -> RetainedTemporalSegmentData {
    let row_count = u32::try_from(rows).expect("synthetic benchmark row count fits u32");
    let mut filecode_values = Vec::with_capacity(rows * std::mem::size_of::<u32>());
    for row in 0..rows {
        filecode_values.extend_from_slice(&((row & 4095) as u32).to_le_bytes());
    }
    let filecode_payload = synthetic_retained_payload(
        row_count,
        CoveEncodingKind::FileCode,
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        filecode_values,
    );

    let bool_values = (0..rows)
        .map(|row| if row & 1 == 0 { 1u8 } else { 0u8 })
        .collect::<Vec<_>>();
    let bool_payload = synthetic_retained_payload(
        row_count,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Bool,
        CovePhysicalKind::Boolean,
        bool_values,
    );

    let mut fixed_values = Vec::with_capacity(rows * 16);
    for row in 0..rows {
        let mut value = [0u8; 16];
        value[0..8].copy_from_slice(&(row as u64).to_le_bytes());
        fixed_values.extend_from_slice(&value);
    }
    let fixed_payload = synthetic_retained_payload(
        row_count,
        CoveEncodingKind::PlainFixed,
        CoveLogicalType::Uuid,
        CovePhysicalKind::FixedBytes,
        fixed_values,
    );

    let mut varbytes_values = Vec::with_capacity(rows * 12);
    for row in 0..rows {
        let value = format!("v{}", row & 1023);
        varbytes_values.extend_from_slice(&(value.len() as u32).to_le_bytes());
        varbytes_values.extend_from_slice(value.as_bytes());
    }
    let varbytes_payload = synthetic_retained_payload(
        row_count,
        CoveEncodingKind::VarBytes,
        CoveLogicalType::Utf8,
        CovePhysicalKind::VarBytes,
        varbytes_values,
    );

    let local_codebook_payload = LocalCodebookPayload {
        values: LocalCodebookValues::FileCode((0..64u32).collect()),
        indexes: LocalIndexPayload::BitPacked(
            BitPackedPayload::pack(
                &(0..rows).map(|row| (row & 63) as u64).collect::<Vec<_>>(),
                6,
            )
            .expect("synthetic local-codebook indexes"),
        ),
    };
    let local_codebook_payload = synthetic_retained_payload(
        row_count,
        CoveEncodingKind::LocalCodebook,
        CoveLogicalType::Utf8,
        CovePhysicalKind::FileCode,
        local_codebook_payload.encode(),
    );

    RetainedTemporalSegmentData {
        header: TemporalSegmentHeaderV1 {
            segment_id: 77,
            object_type_id: 1,
            time_range_start_us: 0,
            time_range_end_us: i64::from(row_count.saturating_sub(1)),
            csn_min: 0,
            csn_max: u64::from(row_count.saturating_sub(1)),
            row_count,
            morsel_count: 1,
            morsel_row_count: row_count,
            column_count: 1,
            row_directory_offset: 0,
            column_directory_offset: 0,
            page_index_offset: 0,
            data_offset: 0,
            flags: 0,
            checksum: 0,
        },
        rows: (0..rows).map(synthetic_temporal_row).collect(),
        property_columns: vec![
            synthetic_retained_property_column(
                1,
                CoveLogicalType::Utf8,
                CovePhysicalKind::FileCode,
                row_count,
                filecode_payload,
            ),
            synthetic_retained_property_column(
                2,
                CoveLogicalType::Bool,
                CovePhysicalKind::Boolean,
                row_count,
                bool_payload,
            ),
            synthetic_retained_property_column(
                3,
                CoveLogicalType::Uuid,
                CovePhysicalKind::FixedBytes,
                row_count,
                fixed_payload,
            ),
            synthetic_retained_property_column(
                4,
                CoveLogicalType::Utf8,
                CovePhysicalKind::VarBytes,
                row_count,
                varbytes_payload,
            ),
            synthetic_retained_property_column(
                5,
                CoveLogicalType::Utf8,
                CovePhysicalKind::FileCode,
                row_count,
                local_codebook_payload,
            ),
        ],
    }
}

fn synthetic_retained_payload(
    row_count: u32,
    encoding_kind: CoveEncodingKind,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    values: Vec<u8>,
) -> RetainedColumnPagePayloadV1 {
    let payload_bytes = ColumnPagePayloadV1::build_single_node(
        row_count,
        encoding_kind,
        logical_type,
        physical_kind,
        None,
        values,
    )
    .expect("synthetic native object page payload");
    RetainedColumnPagePayloadV1::parse(RetainedBytes::from_vec(payload_bytes))
        .expect("synthetic retained page payload")
}

fn synthetic_retained_property_column(
    column_id: u32,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
    row_count: u32,
    payload: RetainedColumnPagePayloadV1,
) -> RetainedTemporalPropertyColumn {
    let payload_len = payload.data.as_slice().len();
    RetainedTemporalPropertyColumn {
        directory: synthetic_property_directory(column_id, logical_type, physical_kind),
        page_index: cove_core::page::ColumnPageIndex {
            entries: Vec::new(),
        },
        pages: vec![RetainedTemporalPropertyPage {
            index_entry: ColumnPageIndexEntryV1 {
                column_id,
                morsel_id: 0,
                row_count,
                non_null_count: row_count,
                null_count: 0,
                encoding_root: 0,
                page_offset: 0,
                page_length: payload_len as u64,
                uncompressed_length: payload_len as u64,
                stats_ref: 0,
                flags: 0,
                checksum: 0,
            },
            payload: Some(payload),
        }],
    }
}

fn synthetic_property_directory(
    column_id: u32,
    logical_type: CoveLogicalType,
    physical_kind: CovePhysicalKind,
) -> TableColumnDirectoryEntryV1 {
    let mut bytes = [0u8; cove_core::segment::TABLE_COLUMN_DIRECTORY_ENTRY_LEN];
    bytes[0..4].copy_from_slice(&column_id.to_le_bytes());
    bytes[4..6].copy_from_slice(&(logical_type as u16).to_le_bytes());
    bytes[6] = physical_kind as u8;
    let crc = checksum::crc32c(&bytes);
    bytes[48..52].copy_from_slice(&crc.to_le_bytes());
    TableColumnDirectoryEntryV1::parse(&bytes).expect("synthetic property column directory")
}

fn synthetic_temporal_row(row: usize) -> TemporalRowEntryV1 {
    let mut goid = [0u8; 16];
    goid[..8].copy_from_slice(&(row as u64).to_le_bytes());
    let mut record_id = [0u8; 16];
    record_id[..8].copy_from_slice(&(row as u64).wrapping_mul(17).to_le_bytes());
    TemporalRowEntryV1 {
        timestamp_us: row as i64,
        csn: row as u64,
        branch_key: 0,
        goid,
        record_id,
        record_kind: RecordKind::Delta,
        prev_ref: None,
    }
}

fn benchmark_object_kernel_execution_report() {
    let bytes = include_bytes!("../../../conformance/accept/cove_o_temporal_valid.cove");
    benchmark_object_kernel_case("direct_projection", bytes, "Thing.select(active)");
    benchmark_object_kernel_case(
        "typed_predicate",
        bytes,
        "Thing.where(active == true).select(active)",
    );
}

fn benchmark_object_kernel_case(label: &str, bytes: &[u8], query: &str) {
    let started = Instant::now();
    let kernel = parse_resolve_plan_build_physical_and_execute_query(
        bytes,
        query,
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        PhysicalPlanOptions::default(),
        ExecutionOptions::default(),
        KernelExecutionOptions {
            mode: KernelExecutionMode::CompareWithMaterialized,
            redact_exact_counters: false,
            ..KernelExecutionOptions::default()
        },
        validation_options(),
    )
    .expect("object kernel benchmark query should execute");
    let elapsed_us = started.elapsed().as_micros();
    let report = &kernel.kernel_report;
    let counters = &report.counters;
    black_box(&kernel);
    println!(
        "kernel_execution object_kernel case={} rows_scanned={} rows_after_bitmap={} rows_pruned_by_bitmap={} rows_after_selection_vector={} rows_pruned_by_selection_vector={} output_rows={} elapsed_us={} rows_per_us={:.3} coded_predicate_rows={} coded_predicate_pct={:.3} typed_predicate_rows={} residual_rows_checked={} residual_rate_per_1k_rows={:.3} dictionary_lookups={} bytes_touched={} scratch_high_water_bytes={} final_materialization_rows={} compared_with_materialized={} materialized_fingerprint_present={} kernel_fingerprint_present={} fallback_reason={:?} authority_state={:?}",
        label,
        counters.rows_scanned,
        counters.rows_after_bitmap,
        report.metrics.rows_pruned_by_bitmap,
        counters.rows_after_selection_vector,
        report.metrics.rows_pruned_by_selection_vector,
        counters.output_rows,
        elapsed_us,
        per_us(counters.rows_scanned, elapsed_us),
        counters.coded_predicate_rows,
        percent(counters.coded_predicate_rows, counters.rows_scanned),
        counters.typed_predicate_rows,
        counters.residual_rows_checked,
        per_1k(counters.residual_rows_checked, counters.rows_scanned),
        counters.dictionary_lookups_at_materialization,
        counters.bytes_touched_estimate,
        counters.scratch_high_water_bytes,
        report.metrics.final_materialization_rows,
        report.compared_with_materialized,
        report.materialized_fingerprint.is_some(),
        report.kernel_fingerprint.is_some(),
        report.fallback_reason,
        report.optimization_authority.state
    );
}

fn benchmark_association_report() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_association_file(),
        "Person.where(exists(either(association(CustomerPlacedOrder)))).select(active)",
        ParseOptions::default(),
        json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .expect("association benchmark query should plan");
    let rows = (0..250_000usize)
        .map(|index| MaterializedAssociationRow {
            dataset_file_ordinal: None,
            dataset_file_source: None,
            dataset_file_id: None,
            output_grain: OutputGrain::AssociationState,
            change: None,
            object_type_id: 7,
            association_type: Some("CustomerPlacedOrder".into()),
            branch_key: 0,
            goid: format!("edge-{index}"),
            record_id: format!("record-{index}"),
            source_goid: Some(format!("source-{}", index % 50_000)),
            target_goid: Some(format!("target-{}", index % 125_000)),
            timestamp_us: index as i64,
            csn: index as u64,
            record_kind: "baseline".into(),
            tombstone_status: "live".into(),
            properties: BTreeMap::new(),
            property_ids: BTreeMap::new(),
            redacted_properties: BTreeSet::new(),
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    let report = AssociationOptimizationReport::for_plan(&planned, &rows);
    let elapsed_us = started.elapsed().as_micros();
    let fast_path_candidates = report.semi_join_candidates
        + report.anti_join_candidates
        + report.count_fast_path_candidates
        + report.distinct_target_fast_path_candidates
        + report.validity_interval_fast_path_candidates;
    black_box(&report);
    println!(
        "kernel_execution association_report edges={} elapsed_us={} edges_per_us={:.3} fast_path_candidates={} candidate_rate_per_1k_edges={:.3} semi_join_candidates={} anti_join_candidates={} count_fast_path_candidates={} distinct_target_fast_path_candidates={} validity_interval_fast_path_candidates={} endpoint_plan_count={} fallback_count={}",
        report.edge_count,
        elapsed_us,
        per_us(report.edge_count, elapsed_us),
        fast_path_candidates,
        per_1k(fast_path_candidates, report.edge_count),
        report.semi_join_candidates,
        report.anti_join_candidates,
        report.count_fast_path_candidates,
        report.distinct_target_fast_path_candidates,
        report.validity_interval_fast_path_candidates,
        report.endpoint_plans.len(),
        report.fallback_reasons.len()
    );
}

fn benchmark_evidence_report() {
    let planned = parse_resolve_and_plan_query(
        &minimal_object_with_evidence_index_file(),
        "Person.where(exists(evidence())).select(active)",
        ParseOptions::default(),
        protected_json_resolve_options(),
        PlanOptions::default(),
        validation_options(),
    )
    .expect("evidence benchmark query should plan");
    let index = MapEvidenceIndex {
        mapping_id: "bench-map".into(),
        mapping_version: "2026.06".into(),
        entries: (0..250_000usize)
            .map(|row| {
                let mut operation_metadata = BTreeMap::new();
                operation_metadata.insert("operation_target".into(), json!("object"));
                MapEvidenceEntry {
                    source_id: format!("source-{}", row % 512),
                    source_row_identity: format!("source-row-{row}"),
                    rule_id: "rule-object".into(),
                    assertion_id: format!("assertion-{row}"),
                    output_object_id: format!("object-{}", row % 50_000),
                    observed_schema_fingerprint: None,
                    observed_snapshot_digest: None,
                    operation_metadata,
                }
            })
            .collect(),
    };

    let started = Instant::now();
    let report = EvidenceOptimizationReport::for_plan(&planned, Some(&index));
    let elapsed_us = started.elapsed().as_micros();
    let indexed_entries = report
        .index_reports
        .iter()
        .map(|grain| grain.indexed_entries)
        .sum::<usize>();
    let fallback_entries = report
        .index_reports
        .iter()
        .map(|grain| grain.fallback_entries)
        .sum::<usize>();
    let fast_path_candidates =
        report.existence_fast_path_candidates + report.count_fast_path_candidates;
    black_box(&report);
    println!(
        "kernel_execution evidence_report entries={} elapsed_us={} entries_per_us={:.3} fast_path_candidates={} candidate_rate_per_1k_entries={:.3} existence_candidates={} count_candidates={} existence_exact={} count_exact={} grain_index_count={} indexed_entries={} fallback_entries={} fallback_rate_per_1k_entries={:.3} hidden_entry_filtering_applied={} target_index_kind_count={} fallback_count={}",
        report.evidence_entry_count,
        elapsed_us,
        per_us(report.evidence_entry_count, elapsed_us),
        fast_path_candidates,
        per_1k(fast_path_candidates, report.evidence_entry_count),
        report.existence_fast_path_candidates,
        report.count_fast_path_candidates,
        report.existence_fast_path_exact,
        report.count_fast_path_exact,
        report.index_reports.len(),
        indexed_entries,
        fallback_entries,
        per_1k(fallback_entries, report.evidence_entry_count),
        report.hidden_entry_filtering_applied,
        report.target_index_kinds.len(),
        report.fallback_reasons.len()
    );
}

fn per_us(count: usize, elapsed_us: u128) -> f64 {
    count as f64 / elapsed_us.max(1) as f64
}

fn per_1k(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 * 1_000.0 / total as f64
    }
}

fn percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 * 100.0 / total as f64
    }
}

fn json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveQlOutputMode::JsonRows),
        ..ResolveOptions::default()
    }
}

fn protected_json_resolve_options() -> ResolveOptions {
    ResolveOptions {
        output_mode: Some(CoveQlOutputMode::JsonRows),
        security: SecurityContext {
            metadata_disclosure_policy: coveql::MetadataDisclosurePolicy::AllowProtected,
            ..SecurityContext::default()
        },
        ..ResolveOptions::default()
    }
}

fn validation_options() -> ValidationOptions {
    ValidationOptions {
        semantic: true,
        ..ValidationOptions::default()
    }
}

fn minimal_object_with_evidence_index_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![ObjectTypeEntryV1 {
            object_type_id: 1,
            type_name: "Person".into(),
            flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
            properties: vec![PropertyEntryV1 {
                property_id: 1,
                property_name: "active".into(),
                logical_type: CoveLogicalType::Bool,
                physical_kind: CovePhysicalKind::Boolean,
                nullable: false,
                collation_id: 0,
                flags: 0,
            }],
        }],
    };
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE | FEATURE_SEMANTIC_MAP;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: catalog.types.len() as u64,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().expect("catalog serialization"),
    });
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapSourceCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "sources": [{
                "source_id": "crm.customers",
                "schema_fingerprint": "schema-v1",
                "snapshot_digest": "digest-v1",
                "row_identity_rules": ["customer_id"],
                "replay_claimed": true
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapIdentityRuleCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "identity_rules": [{
                "rule_id": "person_identity",
                "object_type": "Person",
                "semantic_role": "subject",
                "confidence_class": "authoritative",
                "candidate_only": false,
                "property_conflicts_declared": true,
                "function_ids": [],
                "join_keys": [{
                    "role_id": "customer_id",
                    "source_column": "customer_id",
                    "logical_type": "utf8",
                    "canonicalization": "identity",
                    "null_policy": "reject",
                    "ordering": "asc"
                }]
            }],
            "do_not_merge": []
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapRowSemanticsCatalog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "rules": [{
                "rule_id": "upsert_person",
                "source_id": "crm.customers",
                "identity_rule_id": "person_identity",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "evidence"],
                "function_ids": [],
                "output_assertion_ids": ["assert_person"],
                "association_endpoints": []
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapAssertionLog,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "assertions": [{
                "assertion_id": "assert_person",
                "output_object_id": "goid:person:1"
            }]
        }),
    );
    push_embedded_map_section(
        &mut writer,
        SectionKind::MapEvidenceIndex,
        json!({
            "mapping_id": "customer-map",
            "mapping_version": "2026.05",
            "entries": [{
                "source_id": "crm.customers",
                "source_row_identity": "customer_id=1",
                "rule_id": "upsert_person",
                "assertion_id": "assert_person",
                "output_object_id": "goid:person:1",
                "observed_schema_fingerprint": "schema-v1",
                "observed_snapshot_digest": "digest-v1",
                "operation_target": "object",
                "object_type": "Person"
            }]
        }),
    );
    writer.write().expect("minimal evidence COVE file")
}

fn minimal_object_with_association_file() -> Vec<u8> {
    let catalog = ObjectTypeCatalog {
        flags: 0,
        types: vec![
            ObjectTypeEntryV1 {
                object_type_id: 1,
                type_name: "Person".into(),
                flags: OBJECT_TYPE_FLAG_ENTITY_OBJECT,
                properties: vec![PropertyEntryV1 {
                    property_id: 1,
                    property_name: "active".into(),
                    logical_type: CoveLogicalType::Bool,
                    physical_kind: CovePhysicalKind::Boolean,
                    nullable: false,
                    collation_id: 0,
                    flags: 0,
                }],
            },
            ObjectTypeEntryV1 {
                object_type_id: 7,
                type_name: "CustomerPlacedOrder".into(),
                flags: OBJECT_TYPE_FLAG_ASSOCIATION_OBJECT,
                properties: vec![
                    PropertyEntryV1 {
                        property_id: 11,
                        property_name: "source_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_FROM_GOID,
                    },
                    PropertyEntryV1 {
                        property_id: 12,
                        property_name: "target_goid".into(),
                        logical_type: CoveLogicalType::Uuid,
                        physical_kind: CovePhysicalKind::FixedBytes,
                        nullable: false,
                        collation_id: 0,
                        flags: PROPERTY_FLAG_ASSOCIATION_TO_GOID,
                    },
                ],
            },
        ],
    };
    write_catalog(catalog)
}

fn write_catalog(catalog: ObjectTypeCatalog) -> Vec<u8> {
    let mut writer = MinimalCoveWriter::new();
    writer.primary_profile = PrimaryProfile::ObjectTemporal as u8;
    writer.required_features = FEATURE_OBJECT_PROFILE;
    writer.sections.push(SectionPayload {
        section_kind: SectionKind::ObjectTypeCatalog as u16,
        profile: PrimaryProfile::ObjectTemporal as u8,
        flags: 0,
        item_count: catalog.types.len() as u64,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_OBJECT_PROFILE,
        optional_features: 0,
        data: catalog.serialize().expect("catalog serialization"),
    });
    writer.write().expect("minimal COVE file")
}

fn push_embedded_map_section(
    writer: &mut MinimalCoveWriter,
    section_kind: SectionKind,
    mut value: Value,
) {
    if let Value::Object(object) = &mut value {
        object.insert(
            "schema_id".into(),
            Value::String("org.coveformat.covemap.v2".into()),
        );
        object.insert(
            "section_id".into(),
            Value::Number((section_kind as u16).into()),
        );
    }
    writer.sections.push(SectionPayload {
        section_kind: section_kind as u16,
        profile: PrimaryProfile::SemanticMapping as u8,
        flags: 0,
        item_count: 1,
        row_count: 0,
        compression: CompressionCodec::None as u8,
        alignment_log2: 0,
        required_features: FEATURE_SEMANTIC_MAP,
        optional_features: 0,
        data: serde_json::to_vec_pretty(&value).expect("map section serialization"),
    });
}

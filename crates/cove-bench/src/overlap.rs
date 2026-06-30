use super::*;

pub(super) const OVERLAP_STRESS_SOURCE_COUNT: usize = 8;

pub(super) struct OverlapStressGenerated {
    sources: Vec<PathBuf>,
    parquet_sources: Vec<PathBuf>,
    unique_parquet: PathBuf,
    source_csv_bytes: u64,
    source_parquet_bundle_bytes: u64,
    unique_parquet_bytes: u64,
    unique_payload_bytes: u64,
    duplicate_payload_bytes: u64,
    shared_row_count: usize,
    unique_entity_count: usize,
}

pub(super) fn run_overlap_stress_case(corpus: &Path) -> Result<Value, String> {
    let root = corpus.join("semantic-map-builds").join("overlap-stress");
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let row_count = overlap_stress_row_count(corpus);
    let map_path = root.join("overlap_stress.covemap");
    let map_bytes = overlap_stress_covemap(OVERLAP_STRESS_SOURCE_COUNT)?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let generated = generate_overlap_stress_sources(&root, row_count, OVERLAP_STRESS_SOURCE_COUNT)?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &generated.sources, options)
        .map_err(|err| format!("overlap stress map build failed: {err}"))?;
    let elapsed = start.elapsed().as_nanos();
    let uncompressed_out_dir = root.join("bundle-uncompressed");
    let mut uncompressed_options = MapBuildOptions::new(&uncompressed_out_dir);
    uncompressed_options.force = true;
    uncompressed_options.verify = true;
    uncompressed_options.publish_covm = true;
    uncompressed_options.section_compression = MapBuildSectionCompression::None;
    let uncompressed_start = Instant::now();
    let uncompressed_result = build_from_paths(&map_path, &generated.sources, uncompressed_options)
        .map_err(|err| format!("overlap stress uncompressed map build failed: {err}"))?;
    let uncompressed_elapsed = uncompressed_start.elapsed().as_nanos();
    let expanded_out_dir = root.join("bundle-expanded");
    let mut expanded_options = MapBuildOptions::new(&expanded_out_dir);
    expanded_options.force = true;
    expanded_options.verify = true;
    expanded_options.publish_covm = true;
    expanded_options.evidence_encoding = MapEvidenceEncoding::Expanded;
    let expanded_start = Instant::now();
    let expanded_result = build_from_paths(&map_path, &generated.sources, expanded_options)
        .map_err(|err| format!("overlap stress expanded map build failed: {err}"))?;
    let expanded_elapsed = expanded_start.elapsed().as_nanos();
    let object_bytes = result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let expanded_object_bytes = expanded_result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let uncompressed_object_bytes = uncompressed_result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let projection_bytes = result
        .manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let index_bytes = result
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = result
        .manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let uncompressed_total_bundle_bytes = directory_size(&uncompressed_out_dir)?;
    let expanded_total_bundle_bytes = directory_size(&expanded_out_dir)?;
    let compact_evidence_index_bytes = result
        .manifest
        .pointer("/evidence/emitted_index_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let expanded_evidence_json_bytes = result
        .manifest
        .pointer("/evidence/expanded_json_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_estimated_saved_bytes = result
        .manifest
        .pointer("/evidence/estimated_saved_bytes")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let section_compression_saved_bytes = result
        .manifest
        .pointer("/compression_summary/saved_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let section_compression_uncompressed_bytes = result
        .manifest
        .pointer("/compression_summary/uncompressed_section_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let section_compression_emitted_bytes = result
        .manifest
        .pointer("/compression_summary/emitted_section_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let section_compression_compressed_section_count = result
        .manifest
        .pointer("/compression_summary/compressed_section_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let object_count = result
        .manifest
        .pointer("/counts/object_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let property_value_count = result
        .manifest
        .pointer("/counts/property_value_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_entry_count = result
        .manifest
        .pointer("/counts/evidence_entry_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source_csv_bytes = generated.source_csv_bytes;
    let source_parquet_bundle_bytes = generated.source_parquet_bundle_bytes;
    let duplicate_payload_bytes = generated.duplicate_payload_bytes;
    let artifact_sizes = json!({
        "cove_bytes": object_bytes,
        "parquet_bytes": source_parquet_bundle_bytes,
        "orc_bytes": 0,
        "covx_bytes": 0,
        "cove_o_bytes": object_bytes,
        "cove_t_projection_bytes": projection_bytes,
        "cove_i_bytes": index_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "uncompressed_cove_o_bytes": uncompressed_object_bytes,
        "uncompressed_total_bundle_bytes": uncompressed_total_bundle_bytes,
        "expanded_cove_o_bytes": expanded_object_bytes,
        "expanded_total_bundle_bytes": expanded_total_bundle_bytes,
    });
    let metrics = json_object(vec![
        ("planning_ns", json!(0)),
        ("scan_ns", json!(elapsed)),
        (
            "end_to_end_ns",
            json!(elapsed + uncompressed_elapsed + expanded_elapsed),
        ),
        ("compressed_build_time_ns", json!(elapsed)),
        ("elapsed_time_ns", json!(elapsed)),
        ("build_time_ns", json!(elapsed)),
        ("uncompressed_build_time_ns", json!(uncompressed_elapsed)),
        ("expanded_build_time_ns", json!(expanded_elapsed)),
        ("rows_materialized", json!(object_count)),
        ("source_table_count", json!(OVERLAP_STRESS_SOURCE_COUNT)),
        ("row_count", json!(row_count)),
        ("overlap_fraction", json!(1.0)),
        ("conflict_policy", json!("source_priority_wins")),
        ("source_csv_bytes", json!(source_csv_bytes)),
        ("source_bytes", json!(source_csv_bytes)),
        (
            "source_parquet_bundle_bytes",
            json!(source_parquet_bundle_bytes),
        ),
        (
            "unique_parquet_bytes",
            json!(generated.unique_parquet_bytes),
        ),
        (
            "unique_payload_bytes",
            json!(generated.unique_payload_bytes),
        ),
        ("duplicate_payload_bytes", json!(duplicate_payload_bytes)),
        (
            "duplicate_payload_ratio",
            json!(ratio(
                duplicate_payload_bytes,
                generated.unique_payload_bytes
            )),
        ),
        ("cove_o_bytes", json!(object_bytes)),
        ("compressed_cove_o_bytes", json!(object_bytes)),
        (
            "uncompressed_cove_o_bytes",
            json!(uncompressed_object_bytes),
        ),
        ("compact_cove_o_bytes", json!(object_bytes)),
        (
            "section_compression_saved_bytes",
            json!(section_compression_saved_bytes),
        ),
        (
            "section_compression_uncompressed_bytes",
            json!(section_compression_uncompressed_bytes),
        ),
        (
            "section_compression_emitted_bytes",
            json!(section_compression_emitted_bytes),
        ),
        (
            "section_compression_compressed_section_count",
            json!(section_compression_compressed_section_count),
        ),
        (
            "section_compression_ratio",
            json!(ratio(object_bytes, uncompressed_object_bytes)),
        ),
        ("expanded_cove_o_bytes", json!(expanded_object_bytes)),
        (
            "compact_vs_expanded_cove_o_ratio",
            json!(ratio(object_bytes, expanded_object_bytes)),
        ),
        (
            "expanded_vs_compact_cove_o_ratio",
            json!(ratio(expanded_object_bytes, object_bytes)),
        ),
        (
            "compact_evidence_index_bytes",
            json!(compact_evidence_index_bytes),
        ),
        (
            "expanded_evidence_json_bytes",
            json!(expanded_evidence_json_bytes),
        ),
        (
            "evidence_estimated_saved_bytes",
            json!(evidence_estimated_saved_bytes),
        ),
        (
            "compact_evidence_vs_expanded_json_ratio",
            json!(ratio(
                compact_evidence_index_bytes,
                expanded_evidence_json_bytes
            )),
        ),
        ("projection_bytes", json!(projection_bytes)),
        ("index_bytes", json!(index_bytes)),
        ("covm_bytes", json!(covm_bytes)),
        ("total_bundle_bytes", json!(total_bundle_bytes)),
        (
            "uncompressed_total_bundle_bytes",
            json!(uncompressed_total_bundle_bytes),
        ),
        (
            "expanded_total_bundle_bytes",
            json!(expanded_total_bundle_bytes),
        ),
        (
            "compressed_vs_uncompressed_bundle_ratio",
            json!(ratio(total_bundle_bytes, uncompressed_total_bundle_bytes)),
        ),
        (
            "compact_vs_expanded_bundle_ratio",
            json!(ratio(total_bundle_bytes, expanded_total_bundle_bytes)),
        ),
        (
            "cove_o_vs_source_csv_ratio",
            json!(ratio(object_bytes, source_csv_bytes)),
        ),
        (
            "bundle_vs_source_csv_ratio",
            json!(ratio(total_bundle_bytes, source_csv_bytes)),
        ),
        (
            "cove_o_vs_parquet_bundle_ratio",
            json!(ratio(object_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "bundle_vs_parquet_bundle_ratio",
            json!(ratio(total_bundle_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "cove_o_vs_unique_parquet_ratio",
            json!(ratio(object_bytes, generated.unique_parquet_bytes)),
        ),
        ("object_count", json!(object_count)),
        ("property_value_count", json!(property_value_count)),
        ("evidence_entry_count", json!(evidence_entry_count)),
        (
            "property_values_per_object",
            json!(ratio(property_value_count, object_count)),
        ),
        (
            "evidence_entries_per_object",
            json!(ratio(evidence_entry_count, object_count)),
        ),
        (
            "evidence_to_property_ratio",
            json!(ratio(evidence_entry_count, property_value_count)),
        ),
        ("bytes_read", json!(object_bytes)),
        ("request_count", json!(1)),
        ("fragments_visited", json!(1)),
        ("pages_visited", json!(property_value_count)),
        ("pruning_tightness", json!(0.0)),
        (
            "coverage_cache",
            json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }),
        ),
        (
            "index_use",
            json!({
                "covi_used": index_bytes > 0,
                "lookup_hits": 0,
                "lookup_misses": 0,
                "index_fallbacks": 0,
            }),
        ),
        ("memory_peak_bytes", Value::Null),
        ("artifact_sizes", artifact_sizes),
    ]);
    let cost = json!({
        "comparison": {
            "source_csv_files": path_strings(&generated.sources),
            "source_parquet_files": path_strings(&generated.parquet_sources),
            "unique_parquet_file": display_path(&generated.unique_parquet),
            "semantic_claim": "high-overlap source tables mapped to one object/property state with retained evidence",
            "caveat": "Parquet comparison is a bundle of duplicate source-shaped tables, not a cross-table semantic dedupe format."
        }
    });
    Ok(json!({
        "id": "cove_o_overlap_stress",
        "category": "COVE-O multi-table high-overlap size stress",
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "cove_o", "overlap_stress", "parquet_compare"],
    }))
}

#[derive(Clone, Copy)]
pub(super) struct OverlapScaleSpec {
    id: &'static str,
    category: &'static str,
    row_count: usize,
    source_count: usize,
}

pub(super) fn run_overlap_scale_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    overlap_scale_specs(corpus)
        .into_iter()
        .map(|spec| run_overlap_scale_case(corpus, spec))
        .collect()
}

pub(super) fn overlap_scale_specs(corpus: &Path) -> Vec<OverlapScaleSpec> {
    let base_rows = overlap_stress_row_count(corpus);
    let large_rows = base_rows.saturating_mul(4);
    vec![
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_1_table",
            category: "COVE-O overlap scale baseline: one source table",
            row_count: base_rows,
            source_count: 1,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_2_tables",
            category: "COVE-O overlap scale: two duplicate source tables",
            row_count: base_rows,
            source_count: 2,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_4_tables",
            category: "COVE-O overlap scale: four duplicate source tables",
            row_count: base_rows,
            source_count: 4,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_8_tables",
            category: "COVE-O overlap scale: eight duplicate source tables",
            row_count: base_rows,
            source_count: 8,
        },
        OverlapScaleSpec {
            id: "cove_o_overlap_scale_8_tables_large",
            category: "COVE-O overlap scale: eight duplicate source tables at larger row count",
            row_count: large_rows,
            source_count: 8,
        },
    ]
}

pub(super) fn run_overlap_scale_case(
    corpus: &Path,
    spec: OverlapScaleSpec,
) -> Result<Value, String> {
    let root = corpus
        .join("semantic-map-builds")
        .join("overlap-scale")
        .join(spec.id);
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let map_path = root.join("overlap_scale.covemap");
    let map_bytes = overlap_stress_covemap(spec.source_count)?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let generated = generate_overlap_stress_sources(&root, spec.row_count, spec.source_count)?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &generated.sources, options)
        .map_err(|err| format!("{} map build failed: {err}", spec.id))?;
    let elapsed = start.elapsed().as_nanos();
    let cove_o_bytes = result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cove_t_bytes = result
        .manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covi_bytes = result
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = result
        .manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let object_count = result
        .manifest
        .pointer("/counts/object_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let property_value_count = result
        .manifest
        .pointer("/counts/property_value_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_entry_count = result
        .manifest
        .pointer("/counts/evidence_entry_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source_csv_bytes = generated.source_csv_bytes;
    let source_parquet_bundle_bytes = generated.source_parquet_bundle_bytes;
    let unique_parquet_bytes = generated.unique_parquet_bytes;
    let artifact_sizes = json!({
        "cove_bytes": cove_o_bytes,
        "parquet_bytes": source_parquet_bundle_bytes,
        "orc_bytes": 0,
        "covx_bytes": 0,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_projection_bytes": cove_t_bytes,
        "cove_i_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "unique_parquet_bytes": unique_parquet_bytes,
    });
    let metrics = json_object(vec![
        ("planning_ns", json!(0)),
        ("scan_ns", json!(elapsed)),
        ("end_to_end_ns", json!(elapsed)),
        ("elapsed_time_ns", json!(elapsed)),
        ("build_time_ns", json!(elapsed)),
        ("rows_materialized", json!(object_count)),
        ("source_table_count", json!(spec.source_count)),
        ("row_count", json!(spec.row_count)),
        ("overlap_fraction", json!(1.0)),
        ("source_csv_bytes", json!(source_csv_bytes)),
        ("source_bytes", json!(source_csv_bytes)),
        (
            "source_parquet_bundle_bytes",
            json!(source_parquet_bundle_bytes),
        ),
        ("unique_parquet_bytes", json!(unique_parquet_bytes)),
        (
            "source_parquet_redundancy_ratio",
            json!(ratio(source_parquet_bundle_bytes, unique_parquet_bytes)),
        ),
        (
            "unique_payload_bytes",
            json!(generated.unique_payload_bytes),
        ),
        (
            "duplicate_payload_bytes",
            json!(generated.duplicate_payload_bytes),
        ),
        (
            "duplicate_payload_ratio",
            json!(ratio(
                generated.duplicate_payload_bytes,
                generated.unique_payload_bytes
            )),
        ),
        ("cove_o_bytes", json!(cove_o_bytes)),
        ("cove_t_bytes", json!(cove_t_bytes)),
        ("covi_bytes", json!(covi_bytes)),
        ("covm_bytes", json!(covm_bytes)),
        ("total_bundle_bytes", json!(total_bundle_bytes)),
        (
            "cove_o_vs_source_csv_ratio",
            json!(ratio(cove_o_bytes, source_csv_bytes)),
        ),
        (
            "bundle_vs_source_csv_ratio",
            json!(ratio(total_bundle_bytes, source_csv_bytes)),
        ),
        (
            "cove_o_vs_parquet_bundle_ratio",
            json!(ratio(cove_o_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "bundle_vs_parquet_bundle_ratio",
            json!(ratio(total_bundle_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "cove_o_vs_unique_parquet_ratio",
            json!(ratio(cove_o_bytes, unique_parquet_bytes)),
        ),
        (
            "bundle_vs_unique_parquet_ratio",
            json!(ratio(total_bundle_bytes, unique_parquet_bytes)),
        ),
        ("object_count", json!(object_count)),
        ("property_value_count", json!(property_value_count)),
        ("evidence_entry_count", json!(evidence_entry_count)),
        (
            "property_values_per_object",
            json!(ratio(property_value_count, object_count)),
        ),
        (
            "evidence_entries_per_object",
            json!(ratio(evidence_entry_count, object_count)),
        ),
        ("bytes_read", json!(cove_o_bytes)),
        ("request_count", json!(1)),
        ("fragments_visited", json!(1)),
        ("pages_visited", json!(property_value_count)),
        ("pruning_tightness", json!(0.0)),
        (
            "coverage_cache",
            json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }),
        ),
        (
            "index_use",
            json!({
                "covi_used": covi_bytes > 0,
                "lookup_hits": 0,
                "lookup_misses": 0,
                "index_fallbacks": 0,
            }),
        ),
        ("memory_peak_bytes", Value::Null),
        ("artifact_sizes", artifact_sizes),
    ]);
    let cost = json!({
        "comparison": {
            "source_csv_files": path_strings(&generated.sources),
            "source_parquet_files": path_strings(&generated.parquet_sources),
            "unique_parquet_file": display_path(&generated.unique_parquet),
            "semantic_claim": "same logical object/property state repeated across source tables",
            "caveat": "This is a maximum-overlap synthetic sweep. It demonstrates the crossover curve, not general table-format superiority."
        },
        "manifest": result.manifest,
    });
    Ok(json!({
        "id": spec.id,
        "category": spec.category,
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "cove_o", "overlap_scale", "parquet_compare"],
    }))
}

#[derive(Clone, Copy)]
pub(super) struct OverlapPartialSpec {
    id: &'static str,
    category: &'static str,
    overlap_percent: usize,
    row_count: usize,
    source_count: usize,
}

pub(super) fn run_overlap_partial_cases(corpus: &Path) -> Result<Vec<Value>, String> {
    overlap_partial_specs(corpus)
        .into_iter()
        .map(|spec| run_overlap_partial_case(corpus, spec))
        .collect()
}

pub(super) fn overlap_partial_specs(corpus: &Path) -> Vec<OverlapPartialSpec> {
    let row_count = overlap_stress_row_count(corpus);
    [
        (0, "zero shared entities"),
        (25, "quarter shared entities"),
        (50, "half shared entities"),
        (75, "three-quarter shared entities"),
        (100, "all entities shared"),
    ]
    .into_iter()
    .map(|(overlap_percent, label)| OverlapPartialSpec {
        id: match overlap_percent {
            0 => "cove_o_overlap_partial_0pct",
            25 => "cove_o_overlap_partial_25pct",
            50 => "cove_o_overlap_partial_50pct",
            75 => "cove_o_overlap_partial_75pct",
            100 => "cove_o_overlap_partial_100pct",
            _ => unreachable!("static overlap percentages are exhaustive"),
        },
        category: match overlap_percent {
            0 => "COVE-O partial overlap: zero shared entities",
            25 => "COVE-O partial overlap: quarter shared entities",
            50 => "COVE-O partial overlap: half shared entities",
            75 => "COVE-O partial overlap: three-quarter shared entities",
            100 => "COVE-O partial overlap: all entities shared",
            _ => label,
        },
        overlap_percent,
        row_count,
        source_count: OVERLAP_STRESS_SOURCE_COUNT,
    })
    .collect()
}

pub(super) fn run_overlap_partial_case(
    corpus: &Path,
    spec: OverlapPartialSpec,
) -> Result<Value, String> {
    let root = corpus
        .join("semantic-map-builds")
        .join("overlap-partial")
        .join(spec.id);
    fs::create_dir_all(&root).map_err(|err| format!("cannot create {}: {err}", root.display()))?;
    let map_path = root.join("overlap_partial.covemap");
    let map_bytes = overlap_stress_covemap(spec.source_count)?
        .serialize()
        .map_err(|err| err.to_string())?;
    durable::durable_replace(&map_path, &map_bytes)
        .map_err(|err| format!("cannot publish {}: {err}", map_path.display()))?;
    let generated = generate_overlap_partial_sources(
        &root,
        spec.row_count,
        spec.source_count,
        spec.overlap_percent,
    )?;
    let out_dir = root.join("bundle");
    let mut options = MapBuildOptions::new(&out_dir);
    options.force = true;
    options.verify = true;
    options.publish_covm = true;
    let start = Instant::now();
    let result = build_from_paths(&map_path, &generated.sources, options)
        .map_err(|err| format!("{} map build failed: {err}", spec.id))?;
    let elapsed = start.elapsed().as_nanos();
    let cove_o_bytes = result
        .manifest
        .pointer("/artifacts/object/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cove_t_bytes = result
        .manifest
        .pointer("/artifacts/projections")
        .and_then(Value::as_array)
        .map(|projections| {
            projections
                .iter()
                .filter_map(|projection| projection.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covi_bytes = result
        .manifest
        .pointer("/artifacts/indexes")
        .and_then(Value::as_array)
        .map(|indexes| {
            indexes
                .iter()
                .filter_map(|index| index.get("byte_size").and_then(Value::as_u64))
                .sum::<u64>()
        })
        .unwrap_or(0);
    let covm_bytes = result
        .manifest
        .pointer("/artifacts/covm/byte_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_bundle_bytes = directory_size(&out_dir)?;
    let object_count = result
        .manifest
        .pointer("/counts/object_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let property_value_count = result
        .manifest
        .pointer("/counts/property_value_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let evidence_entry_count = result
        .manifest
        .pointer("/counts/evidence_entry_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let source_csv_bytes = generated.source_csv_bytes;
    let source_parquet_bundle_bytes = generated.source_parquet_bundle_bytes;
    let unique_parquet_bytes = generated.unique_parquet_bytes;
    let source_input_row_count = (spec.row_count as u64).saturating_mul(spec.source_count as u64);
    let artifact_sizes = json!({
        "cove_bytes": cove_o_bytes,
        "parquet_bytes": source_parquet_bundle_bytes,
        "orc_bytes": 0,
        "covx_bytes": 0,
        "cove_o_bytes": cove_o_bytes,
        "cove_t_projection_bytes": cove_t_bytes,
        "cove_i_bytes": covi_bytes,
        "covm_bytes": covm_bytes,
        "total_bundle_bytes": total_bundle_bytes,
        "unique_parquet_bytes": unique_parquet_bytes,
    });
    let metrics = json_object(vec![
        ("planning_ns", json!(0)),
        ("scan_ns", json!(elapsed)),
        ("end_to_end_ns", json!(elapsed)),
        ("elapsed_time_ns", json!(elapsed)),
        ("build_time_ns", json!(elapsed)),
        ("rows_materialized", json!(object_count)),
        ("source_table_count", json!(spec.source_count)),
        ("row_count", json!(spec.row_count)),
        ("source_input_row_count", json!(source_input_row_count)),
        (
            "overlap_fraction",
            json!(spec.overlap_percent as f64 / 100.0),
        ),
        ("overlap_percent", json!(spec.overlap_percent)),
        ("shared_row_count", json!(generated.shared_row_count)),
        (
            "source_unique_rows_per_table",
            json!(spec.row_count.saturating_sub(generated.shared_row_count)),
        ),
        ("unique_entity_count", json!(generated.unique_entity_count)),
        (
            "object_dedupe_ratio",
            json!(ratio(source_input_row_count, object_count)),
        ),
        ("source_csv_bytes", json!(source_csv_bytes)),
        ("source_bytes", json!(source_csv_bytes)),
        (
            "source_parquet_bundle_bytes",
            json!(source_parquet_bundle_bytes),
        ),
        ("unique_parquet_bytes", json!(unique_parquet_bytes)),
        (
            "source_parquet_redundancy_ratio",
            json!(ratio(source_parquet_bundle_bytes, unique_parquet_bytes)),
        ),
        (
            "unique_payload_bytes",
            json!(generated.unique_payload_bytes),
        ),
        (
            "duplicate_payload_bytes",
            json!(generated.duplicate_payload_bytes),
        ),
        (
            "duplicate_payload_ratio",
            json!(ratio(
                generated.duplicate_payload_bytes,
                generated.unique_payload_bytes
            )),
        ),
        ("cove_o_bytes", json!(cove_o_bytes)),
        ("cove_t_bytes", json!(cove_t_bytes)),
        ("covi_bytes", json!(covi_bytes)),
        ("covm_bytes", json!(covm_bytes)),
        ("total_bundle_bytes", json!(total_bundle_bytes)),
        (
            "cove_o_vs_source_csv_ratio",
            json!(ratio(cove_o_bytes, source_csv_bytes)),
        ),
        (
            "bundle_vs_source_csv_ratio",
            json!(ratio(total_bundle_bytes, source_csv_bytes)),
        ),
        (
            "cove_o_vs_parquet_bundle_ratio",
            json!(ratio(cove_o_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "bundle_vs_parquet_bundle_ratio",
            json!(ratio(total_bundle_bytes, source_parquet_bundle_bytes)),
        ),
        (
            "cove_o_vs_unique_parquet_ratio",
            json!(ratio(cove_o_bytes, unique_parquet_bytes)),
        ),
        (
            "bundle_vs_unique_parquet_ratio",
            json!(ratio(total_bundle_bytes, unique_parquet_bytes)),
        ),
        ("object_count", json!(object_count)),
        ("property_value_count", json!(property_value_count)),
        ("evidence_entry_count", json!(evidence_entry_count)),
        (
            "property_values_per_object",
            json!(ratio(property_value_count, object_count)),
        ),
        (
            "evidence_entries_per_object",
            json!(ratio(evidence_entry_count, object_count)),
        ),
        ("bytes_read", json!(cove_o_bytes)),
        ("request_count", json!(1)),
        ("fragments_visited", json!(1)),
        ("pages_visited", json!(property_value_count)),
        ("pruning_tightness", json!(0.0)),
        (
            "coverage_cache",
            json!({
                "hits": 0,
                "misses": 0,
                "entries_loaded": 0,
            }),
        ),
        (
            "index_use",
            json!({
                "covi_used": covi_bytes > 0,
                "lookup_hits": 0,
                "lookup_misses": 0,
                "index_fallbacks": 0,
            }),
        ),
        ("memory_peak_bytes", Value::Null),
        ("artifact_sizes", artifact_sizes),
    ]);
    let cost = json!({
        "comparison": {
            "source_csv_files": path_strings(&generated.sources),
            "source_parquet_files": path_strings(&generated.parquet_sources),
            "unique_parquet_file": display_path(&generated.unique_parquet),
            "semantic_claim": "a controlled fraction of source rows map to shared logical objects",
            "caveat": "This is a synthetic overlap sweep. It isolates overlap effects but does not model every real data-quality or schema-divergence cost."
        },
        "manifest": result.manifest,
    });
    Ok(json!({
        "id": spec.id,
        "category": spec.category,
        "status": "measured",
        "metrics": metrics,
        "cost": cost,
        "optional_features": ["cove_map", "map_build", "cove_o", "overlap_partial", "parquet_compare"],
    }))
}

pub(super) fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

pub(super) fn overlap_stress_row_count(corpus: &Path) -> usize {
    let profile = fs::read(corpus.join("corpus.lock.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("profile")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    match profile.as_deref() {
        Some("publication") => 4_096,
        Some("standard") => 1_024,
        _ => 512,
    }
}

pub(super) fn generate_overlap_stress_sources(
    root: &Path,
    row_count: usize,
    source_count: usize,
) -> Result<OverlapStressGenerated, String> {
    let mut sources = Vec::with_capacity(source_count);
    let mut parquet_sources = Vec::with_capacity(source_count);
    let mut source_csv_bytes = 0u64;
    let mut source_parquet_bundle_bytes = 0u64;
    let mut unique_payload_bytes = 0u64;
    let mut total_payload_bytes = 0u64;
    for source_index in 0..source_count {
        let csv_path = root.join(format!("overlap_source_{source_index:02}.csv"));
        let csv = overlap_stress_csv(row_count, &mut total_payload_bytes);
        if source_index == 0 {
            unique_payload_bytes = total_payload_bytes;
        }
        fs::write(&csv_path, csv.as_bytes())
            .map_err(|err| format!("cannot write {}: {err}", csv_path.display()))?;
        source_csv_bytes = source_csv_bytes
            .checked_add(csv.len() as u64)
            .ok_or_else(|| "overlap source CSV size overflow".to_string())?;
        sources.push(csv_path);

        let parquet_path = root.join(format!("overlap_source_{source_index:02}.parquet"));
        write_parquet_file(&parquet_path, &overlap_stress_batch(row_count)?)?;
        let parquet_bytes = fs::metadata(&parquet_path)
            .map_err(|err| format!("cannot stat {}: {err}", parquet_path.display()))?
            .len();
        source_parquet_bundle_bytes = source_parquet_bundle_bytes
            .checked_add(parquet_bytes)
            .ok_or_else(|| "overlap source Parquet size overflow".to_string())?;
        parquet_sources.push(parquet_path);
    }
    let unique_parquet = root.join("overlap_unique.parquet");
    write_parquet_file(&unique_parquet, &overlap_stress_batch(row_count)?)?;
    let unique_parquet_bytes = fs::metadata(&unique_parquet)
        .map_err(|err| format!("cannot stat {}: {err}", unique_parquet.display()))?
        .len();
    let duplicate_payload_bytes = total_payload_bytes.saturating_sub(unique_payload_bytes);
    Ok(OverlapStressGenerated {
        sources,
        parquet_sources,
        unique_parquet,
        source_csv_bytes,
        source_parquet_bundle_bytes,
        unique_parquet_bytes,
        unique_payload_bytes,
        duplicate_payload_bytes,
        shared_row_count: row_count,
        unique_entity_count: row_count,
    })
}

pub(super) fn generate_overlap_partial_sources(
    root: &Path,
    row_count: usize,
    source_count: usize,
    overlap_percent: usize,
) -> Result<OverlapStressGenerated, String> {
    let shared_row_count = row_count.saturating_mul(overlap_percent.min(100)) / 100;
    let unique_rows_per_source = row_count.saturating_sub(shared_row_count);
    let unique_entity_count = shared_row_count
        .checked_add(
            source_count
                .checked_mul(unique_rows_per_source)
                .ok_or_else(|| "overlap partial unique entity count overflow".to_string())?,
        )
        .ok_or_else(|| "overlap partial unique entity count overflow".to_string())?;
    let unique_entities = (0..unique_entity_count).collect::<Vec<_>>();
    let mut sources = Vec::with_capacity(source_count);
    let mut parquet_sources = Vec::with_capacity(source_count);
    let mut source_csv_bytes = 0u64;
    let mut source_parquet_bundle_bytes = 0u64;
    let mut total_payload_bytes = 0u64;
    for source_index in 0..source_count {
        let rows = overlap_partial_entity_rows(
            row_count,
            shared_row_count,
            unique_rows_per_source,
            source_index,
        );
        let csv_path = root.join(format!("overlap_source_{source_index:02}.csv"));
        let csv = overlap_partial_csv(&rows, &mut total_payload_bytes);
        fs::write(&csv_path, csv.as_bytes())
            .map_err(|err| format!("cannot write {}: {err}", csv_path.display()))?;
        source_csv_bytes = source_csv_bytes
            .checked_add(csv.len() as u64)
            .ok_or_else(|| "overlap partial source CSV size overflow".to_string())?;
        sources.push(csv_path);

        let parquet_path = root.join(format!("overlap_source_{source_index:02}.parquet"));
        write_parquet_file(&parquet_path, &overlap_partial_batch(&rows)?)?;
        let parquet_bytes = fs::metadata(&parquet_path)
            .map_err(|err| format!("cannot stat {}: {err}", parquet_path.display()))?
            .len();
        source_parquet_bundle_bytes = source_parquet_bundle_bytes
            .checked_add(parquet_bytes)
            .ok_or_else(|| "overlap partial source Parquet size overflow".to_string())?;
        parquet_sources.push(parquet_path);
    }
    let unique_parquet = root.join("overlap_unique.parquet");
    write_parquet_file(&unique_parquet, &overlap_partial_batch(&unique_entities)?)?;
    let unique_parquet_bytes = fs::metadata(&unique_parquet)
        .map_err(|err| format!("cannot stat {}: {err}", unique_parquet.display()))?
        .len();
    let unique_payload_bytes = unique_entities
        .iter()
        .map(|row| overlap_payload_size(*row))
        .sum::<u64>();
    let duplicate_payload_bytes = total_payload_bytes.saturating_sub(unique_payload_bytes);
    Ok(OverlapStressGenerated {
        sources,
        parquet_sources,
        unique_parquet,
        source_csv_bytes,
        source_parquet_bundle_bytes,
        unique_parquet_bytes,
        unique_payload_bytes,
        duplicate_payload_bytes,
        shared_row_count,
        unique_entity_count,
    })
}

pub(super) fn overlap_partial_entity_rows(
    row_count: usize,
    shared_row_count: usize,
    unique_rows_per_source: usize,
    source_index: usize,
) -> Vec<usize> {
    (0..row_count)
        .map(|row| {
            if row < shared_row_count {
                row
            } else {
                shared_row_count
                    + source_index.saturating_mul(unique_rows_per_source)
                    + row.saturating_sub(shared_row_count)
            }
        })
        .collect()
}

pub(super) fn overlap_partial_csv(rows: &[usize], total_payload_bytes: &mut u64) -> String {
    let mut csv = String::from("id,name,email,address,bio,segment,plan,score\n");
    for row in rows {
        let values = overlap_stress_values(*row);
        *total_payload_bytes =
            total_payload_bytes.saturating_add(values.iter().map(|value| value.len() as u64).sum());
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7]
        ));
    }
    csv
}

pub(super) fn overlap_partial_batch(rows: &[usize]) -> Result<RecordBatch, String> {
    let mut ids = Vec::with_capacity(rows.len());
    let mut names = Vec::with_capacity(rows.len());
    let mut emails = Vec::with_capacity(rows.len());
    let mut addresses = Vec::with_capacity(rows.len());
    let mut bios = Vec::with_capacity(rows.len());
    let mut segments = Vec::with_capacity(rows.len());
    let mut plans = Vec::with_capacity(rows.len());
    let mut scores = Vec::with_capacity(rows.len());
    for row in rows {
        let values = overlap_stress_values(*row);
        ids.push(values[0].clone());
        names.push(values[1].clone());
        emails.push(values[2].clone());
        addresses.push(values[3].clone());
        bios.push(values[4].clone());
        segments.push(values[5].clone());
        plans.push(values[6].clone());
        scores.push(overlap_stress_score(*row));
    }
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(ids)) as ArrayRef),
        ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("email", Arc::new(StringArray::from(emails)) as ArrayRef),
        (
            "address",
            Arc::new(StringArray::from(addresses)) as ArrayRef,
        ),
        ("bio", Arc::new(StringArray::from(bios)) as ArrayRef),
        ("segment", Arc::new(StringArray::from(segments)) as ArrayRef),
        ("plan", Arc::new(StringArray::from(plans)) as ArrayRef),
        ("score", Arc::new(Int64Array::from(scores)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())
}

pub(super) fn overlap_payload_size(row: usize) -> u64 {
    overlap_stress_values(row)
        .iter()
        .map(|value| value.len() as u64)
        .sum()
}

pub(super) fn overlap_stress_csv(row_count: usize, total_payload_bytes: &mut u64) -> String {
    let mut csv = String::from("id,name,email,address,bio,segment,plan,score\n");
    for row in 0..row_count {
        let values = overlap_stress_values(row);
        *total_payload_bytes = total_payload_bytes
            .saturating_add(values.iter().map(|value| value.len() as u64).sum::<u64>());
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7]
        ));
    }
    csv
}

pub(super) fn overlap_stress_batch(row_count: usize) -> Result<RecordBatch, String> {
    let mut ids = Vec::with_capacity(row_count);
    let mut names = Vec::with_capacity(row_count);
    let mut emails = Vec::with_capacity(row_count);
    let mut addresses = Vec::with_capacity(row_count);
    let mut bios = Vec::with_capacity(row_count);
    let mut segments = Vec::with_capacity(row_count);
    let mut plans = Vec::with_capacity(row_count);
    let mut scores = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let values = overlap_stress_values(row);
        ids.push(values[0].clone());
        names.push(values[1].clone());
        emails.push(values[2].clone());
        addresses.push(values[3].clone());
        bios.push(values[4].clone());
        segments.push(values[5].clone());
        plans.push(values[6].clone());
        scores.push(overlap_stress_score(row));
    }
    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(StringArray::from(ids)) as ArrayRef),
        ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("email", Arc::new(StringArray::from(emails)) as ArrayRef),
        (
            "address",
            Arc::new(StringArray::from(addresses)) as ArrayRef,
        ),
        ("bio", Arc::new(StringArray::from(bios)) as ArrayRef),
        ("segment", Arc::new(StringArray::from(segments)) as ArrayRef),
        ("plan", Arc::new(StringArray::from(plans)) as ArrayRef),
        ("score", Arc::new(Int64Array::from(scores)) as ArrayRef),
    ])
    .map_err(|err| err.to_string())
}

pub(super) fn overlap_stress_values(row: usize) -> Vec<String> {
    vec![
        format!("cust-{row:08}"),
        format!("Customer-{row:08}"),
        format!("customer-{row:08}@example.internal"),
        format!(
            "{}-Long-Duplicate-Avenue-Suite-{}-Region-{}",
            1000 + row,
            row % 997,
            row % 17
        ),
        overlap_stress_bio(row),
        ["consumer", "commercial", "enterprise", "public-sector"][row % 4].to_string(),
        ["free", "starter", "growth", "scale", "global"][row % 5].to_string(),
        overlap_stress_score(row).to_string(),
    ]
}

pub(super) fn overlap_stress_score(row: usize) -> i64 {
    ((row * 37) % 1000) as i64
}

pub(super) fn overlap_stress_bio(row: usize) -> String {
    let token = format!("profile{row:08}");
    let mut value = String::with_capacity(288);
    for index in 0..18 {
        if index > 0 {
            value.push('-');
        }
        value.push_str(&token);
        value.push_str(match index % 4 {
            0 => "retail-history",
            1 => "support-context",
            2 => "billing-footprint",
            _ => "marketing-consent",
        });
    }
    value
}

pub(super) fn overlap_stress_covemap(source_count: usize) -> Result<CovemapFile, String> {
    let sources = (0..source_count)
        .map(|index| {
            json!({
                "source_id": format!("overlap_source_{index:02}"),
                "row_identity_rules": ["customer_by_id"],
                "source_priority": index,
            })
        })
        .collect::<Vec<_>>();
    let rules = (0..source_count)
        .map(|index| {
            json!({
                "rule_id": format!("overlap_source_{index:02}_customer"),
                "source_id": format!("overlap_source_{index:02}"),
                "identity_rule_id": "customer_by_id",
                "row_semantics_kind": "Object",
                "assertion_kinds": ["object", "property", "evidence"],
                "function_ids": ["identity"],
                "output_assertion_ids": [],
                "association_endpoints": [],
                "property_bindings": overlap_stress_property_bindings(index),
            })
        })
        .collect::<Vec<_>>();
    Ok(CovemapFile {
        header: CovemapHeaderV1::new([0x6f; 16], 0),
        mapping_version: "bench/overlap-stress.v1".into(),
        sections: vec![
            covemap_json_section(
                SectionKind::MapSourceCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "sources": sources,
                }),
            )?,
            covemap_json_section(
                SectionKind::MapFunctionRegistry,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "functions": [{
                        "function_id": "identity",
                        "version": "1",
                        "deterministic": true,
                        "dependency": "pure"
                    }]
                }),
            )?,
            covemap_json_section(
                SectionKind::MapIdentityRuleCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "identity_rules": [{
                        "rule_id": "customer_by_id",
                        "object_type": "Customer",
                        "semantic_role": "subject",
                        "confidence_class": "authoritative",
                        "candidate_only": false,
                        "property_conflicts_declared": true,
                        "function_ids": ["identity"],
                        "join_keys": [{
                            "role_id": "customer_id",
                            "source_column": "id",
                            "logical_type": "utf8",
                            "canonicalization": "identity",
                            "null_policy": "reject",
                            "ordering": "declared"
                        }]
                    }],
                    "do_not_merge": []
                }),
            )?,
            covemap_json_section(
                SectionKind::MapRowSemanticsCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "rules": rules,
                }),
            )?,
            covemap_json_section(
                SectionKind::MapProjectionCatalog,
                json!({
                    "mapping_id": "overlap-stress-map",
                    "mapping_version": "bench/overlap-stress.v1",
                    "projections": [{
                        "projection_id": "overlap_customers.v1",
                        "output_table": "overlap_customers",
                        "row_grain": "one_row_per_object",
                        "anchor": {"object_type": "Customer"},
                        "temporal_mode": {"as_of": "latest_committed"},
                        "multi_value_policy": "reject",
                        "missing_policy": "null",
                        "output_modes": ["json", "arrow", "cove-t"],
                        "columns": [
                            {"name": "id", "logical_type": "utf8", "value": "id"},
                            {"name": "name", "logical_type": "utf8", "value": "name"},
                            {"name": "email", "logical_type": "utf8", "value": "email"},
                            {"name": "address", "logical_type": "utf8", "value": "address"},
                            {"name": "bio", "logical_type": "utf8", "value": "bio"},
                            {"name": "segment", "logical_type": "utf8", "value": "segment"},
                            {"name": "plan", "logical_type": "utf8", "value": "plan"},
                            {"name": "score", "logical_type": "int64", "value": "score"}
                        ]
                    }]
                }),
            )?,
        ],
        postscript: cove_core::artifact::covemap::CovemapPostscriptV1 {
            required_features: FEATURE_SEMANTIC_MAP,
            optional_features: 0,
            file_len: 0,
            header_offset: 0,
            header_length: 0,
            checksum: 0,
        },
    })
}

pub(super) fn overlap_stress_property_bindings(source_index: usize) -> Value {
    Value::Array(
        [
            ("id", "id", "utf8", false),
            ("name", "name", "utf8", false),
            ("email", "email", "utf8", false),
            ("address", "address", "utf8", false),
            ("bio", "bio", "utf8", false),
            ("segment", "segment", "utf8", false),
            ("plan", "plan", "utf8", false),
            ("score", "score", "int64", false),
        ]
        .into_iter()
        .map(|(property_id, source_column, logical_type, nullable)| {
            json!({
                "assertion_id": format!("source_{source_index:02}_{property_id}"),
                "property_id": property_id,
                "property_name": property_id,
                "source_column": source_column,
                "logical_type": logical_type,
                "nullable": nullable,
                "conflict_policy": "source_priority_wins",
            })
        })
        .collect::<Vec<_>>(),
    )
}

pub(super) fn path_strings(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| display_path(path)).collect()
}

pub(super) fn display_path(path: &Path) -> String {
    path.display().to_string()
}

pub(super) fn json_object(entries: Vec<(&'static str, Value)>) -> Value {
    let mut object = serde_json::Map::new();
    for (key, value) in entries {
        object.insert(key.to_string(), value);
    }
    Value::Object(object)
}

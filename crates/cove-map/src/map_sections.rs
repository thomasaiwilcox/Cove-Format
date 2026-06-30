use super::*;

pub(crate) fn map_passthrough_sections(
    file: &CovemapFile,
    materialized: &MaterializedModel,
) -> Result<Vec<SectionPayload>, String> {
    file.sections
        .iter()
        .filter_map(|section| {
            let kind = u16::try_from(section.entry.section_id)
                .ok()
                .and_then(SectionKind::from_u16)?;
            matches!(
                kind,
                SectionKind::MapSourceCatalog
                    | SectionKind::MapFunctionRegistry
                    | SectionKind::MapIdentityRuleCatalog
                    | SectionKind::MapRowSemanticsCatalog
                    | SectionKind::MapProjectionCatalog
                    | SectionKind::MapResolutionCatalog
            )
            .then(|| {
                let data = if kind == SectionKind::MapProjectionCatalog {
                    enriched_projection_catalog_payload(section.payload.as_slice(), materialized)
                } else {
                    Ok(section.payload.clone())
                };
                data.map(|data| map_section(kind, 1, data))
            })
        })
        .collect()
}

fn enriched_projection_catalog_payload(
    payload: &[u8],
    materialized: &MaterializedModel,
) -> Result<Vec<u8>, String> {
    let section = cove_core::profile::cove_map::parse_embedded_section(
        SectionKind::MapProjectionCatalog,
        payload,
    )
    .map_err(|err| format!("cannot parse MAP_PROJECTION_CATALOG for lineage enrichment: {err}"))?;
    let cove_core::profile::cove_map::EmbeddedMapSection::ProjectionCatalog(catalog) = section
    else {
        return Err("MAP_PROJECTION_CATALOG parser returned a non-projection section".into());
    };
    let catalog = project::enrich_projection_catalog_lineage(catalog, &materialized.object_types);
    serde_json::to_vec_pretty(&project::projection_catalog_json_value(&catalog))
        .map_err(|err| format!("cannot encode enriched MAP_PROJECTION_CATALOG: {err}"))
}

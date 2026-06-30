use super::*;

pub(super) fn resolved_fingerprint(
    root: &ResolvedRoot,
    method_chain: &ResolvedMethodChain,
    output_mode: &CoveQlOutputMode,
    temporal: &TemporalContext,
    branch: &BranchContext,
    tombstone: &TombstoneContext,
    profiles: &[CoveQlProfileId],
) -> String {
    let profile_contracts = profiles
        .iter()
        .map(|profile| {
            let contract = crate::coveql_profile_contract(*profile);
            json!({
                "profile_id": profile,
                "profile_version": contract.profile_version,
                "implemented": contract.implemented,
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "core_contract": {
            "language_version": crate::COVEQL_LANGUAGE_VERSION,
            "core_version": crate::COVEQL_CORE_VERSION,
            "profile_contract_version": crate::COVEQL_PROFILE_CONTRACT_VERSION,
        },
        "profiles": profiles,
        "profile_contracts": profile_contracts,
        "root": root,
        "method_chain": method_chain,
        "output_mode": output_mode,
        "temporal": temporal,
        "branch": branch,
        "tombstone": tombstone,
    });
    sha256_hex(
        serde_json::to_string(&value)
            .expect("canonical resolved query serializes")
            .as_bytes(),
    )
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}

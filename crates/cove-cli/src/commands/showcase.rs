fn run_showcase_customer360(
    out_dir: &Path,
    profile: Customer360Profile,
    force: bool,
    json: bool,
) -> Result<(), String> {
    let manifest = generate_customer360(&Customer360Options {
        out_dir: out_dir.to_path_buf(),
        profile,
        force,
    })
    .map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("cannot serialize Customer 360 manifest: {error}"))?
        );
    } else {
        println!(
            "Generated Customer 360 showcase ({}) at {}",
            profile.as_str(),
            out_dir.display()
        );
        println!(
            "Manifest: {}",
            out_dir.join("customer360-manifest.json").display()
        );
        println!("Try next:");
        println!(
            "  cove inspect --queries --performance {}/customers.cove",
            out_dir.display()
        );
        println!("  cove query {}/customers.cove 'table(customers).select(customer_id, full_name, region, tier, score, status, plan, mrr).take(10)'", out_dir.display());
        println!(
            "  python3 {}/notebooks/customer360_analysis.py --input-dir {}",
            out_dir.display(),
            out_dir.display()
        );
    }
    Ok(())
}

fn run_showcase_proof_suite(
    out_dir: &Path,
    profile: Customer360Profile,
    scenario: ProofSuiteScenario,
    force: bool,
    json: bool,
) -> Result<(), String> {
    let manifest = generate_proof_suite(&ProofSuiteOptions {
        out_dir: out_dir.to_path_buf(),
        profile,
        scenario,
        force,
    })
    .map_err(|error| error.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("cannot serialize proof-suite manifest: {error}"))?
        );
    } else {
        println!(
            "Generated COVE-O proof suite ({}, scenario {}) at {}",
            profile.as_str(),
            scenario.as_str(),
            out_dir.display()
        );
        println!(
            "Manifest: {}",
            out_dir.join("proof-suite-manifest.json").display()
        );
        println!("Try next:");
        println!(
            "  cove map doctor --bundle-dir {}/customer360/map-build-bundle",
            out_dir.display()
        );
        println!(
            "  cove map doctor --bundle-dir {}/claims/map-build-bundle",
            out_dir.display()
        );
        println!(
            "  cove map doctor --bundle-dir {}/catalog/map-build-bundle",
            out_dir.display()
        );
    }
    Ok(())
}

fn run_showcase_ai_training(
    out_dir: &Path,
    profile: Customer360Profile,
    force: bool,
    json: bool,
) -> Result<(), String> {
    let manifest = build_ai_training_showcase(out_dir, profile.as_str(), force)
        .map_err(|err| err.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest).map_err(|error| format!(
                "cannot serialize AI training showcase manifest: {error}"
            ))?
        );
    } else {
        println!(
            "Generated COVE-AI training archive showcase ({}) at {}",
            profile.as_str(),
            out_dir.display()
        );
        println!("Archive: {}", out_dir.join("training.coveai").display());
        println!("Manifest: {}", out_dir.join("training.covm").display());
        println!("Try next:");
        println!(
            "  cove ai verify {} --policy-report",
            out_dir.join("training.coveai").display()
        );
        println!(
            "  cove ai stream {} --format hf-jsonl --split train",
            out_dir.join("training.coveai").display()
        );
        println!(
            "  python3 {}/load_archive.py {}/training.coveai",
            out_dir.display(),
            out_dir.display()
        );
    }
    Ok(())
}


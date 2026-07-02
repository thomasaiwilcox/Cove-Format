fn run_digest(args: Vec<String>) -> Result<(), String> {
    if run_digest_verify(args)? {
        Ok(())
    } else {
        Err("digest verification failed".into())
    }
}

fn run_digest_verify(args: Vec<String>) -> Result<bool, String> {
    let mut require = false;
    let mut input = None;
    for arg in args {
        match arg.as_str() {
            "--require" => require = true,
            "-h" | "--help" => {
                println!("usage: cove digest verify <file.cove> [--require]");
                return Ok(true);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown digest option {arg}")),
            _ => {
                if input.replace(PathBuf::from(arg)).is_some() {
                    return Err("expected one <file.cove>".into());
                }
            }
        }
    }
    let input = input.ok_or_else(|| "expected <file.cove>".to_string())?;
    let bytes =
        fs::read(&input).map_err(|error| format!("cannot read {}: {error}", input.display()))?;
    let structural = validate_bytes_with_options(
        &bytes,
        ValidationOptions {
            semantic: false,
            verify_digests: false,
            ..ValidationOptions::default()
        },
    )
    .map_err(|error| format!("cannot validate {}: {error}", input.display()))?;
    let has_manifest = structural
        .validated
        .footer
        .sections
        .iter()
        .any(|entry| entry.section_kind == SectionKind::DigestManifest as u16);
    let (status, success, error) = if !has_manifest {
        ("missing_manifest", !require, None)
    } else {
        match validate_bytes_with_options(
            &bytes,
            ValidationOptions {
                semantic: true,
                verify_digests: true,
                ..ValidationOptions::default()
            },
        ) {
            Ok(_) => ("verified", true, None),
            Err(error) => ("mismatch", false, Some(error.to_string())),
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "file": input.display().to_string(),
            "status": status,
            "require": require,
            "digest_manifest_present": has_manifest,
            "error": error,
        }))
        .map_err(|error| format!("cannot serialize report: {error}"))?
    );
    Ok(success)
}


/// Integration tests for the `omnicompress` CLI binary.
///
/// Verifies that `omnicompress compress <file>` prints valid JSON
/// with tokens_before, tokens_after, and tokens_saved.
#[test]
fn cli_compress_file_reports_savings() {
    use std::process::Command;

    // Build a JSON array large enough to trigger compression
    // (needs to exceed min_chars_to_compress = 600)
    let big = "[".to_string()
        + &(0..60)
            .map(|i| format!(r#"{{"id":{i},"v":"{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
        + "]";

    let f = std::env::temp_dir().join("omnicompress_cli_test.json");
    std::fs::write(&f, &big).expect("write test file");

    let out = Command::new(env!("CARGO_BIN_EXE_omnicompress"))
        .arg("compress")
        .arg(&f)
        .output()
        .expect("run omnicompress binary");

    assert!(
        out.status.success(),
        "binary exited with non-zero status: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tokens_before"),
        "stdout missing tokens_before: {stdout}"
    );
    assert!(
        stdout.contains("tokens_after"),
        "stdout missing tokens_after: {stdout}"
    );
    assert!(
        stdout.contains("tokens_saved"),
        "stdout missing tokens_saved: {stdout}"
    );

    // Clean up
    let _ = std::fs::remove_file(&f);
}

/// Integration test for `omnicompress eval <dir>`.
///
/// Writes a temp dir with one JSON file (a `Vec<Block>` with a big tool block)
/// and asserts the output contains a ratio and `roundtrip_ok`.
#[test]
fn cli_eval_dir_reports_ratio_and_roundtrip() {
    use std::process::Command;

    // Build a JSON array large enough to trigger compression (>600 chars, >20 items).
    let big_json_content = "[".to_string()
        + &(0..60)
            .map(|i| format!(r#"{{"id":{i},"name":"item-{i}","score":{i}.0,"desc":"padding-{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
        + "]";

    // Construct a Vec<Block> as JSON: one big tool block + 6 recent prose blocks.
    // Block serialisation: {"role":"User","content":"...","tool_name":"Bash"}
    let mut blocks = vec![
        serde_block("User", &big_json_content, Some("Bash")),
    ];
    for i in 0..6 {
        blocks.push(serde_block("Assistant", &format!("ok {i}"), None));
    }
    let sample_json = format!("[{}]", blocks.join(","));

    // Write a temp dir with one sample file.
    let tmp_dir = std::env::temp_dir().join("omnicompress_eval_test_dir");
    std::fs::create_dir_all(&tmp_dir).expect("create temp dir");
    let sample_path = tmp_dir.join("sample.json");
    std::fs::write(&sample_path, &sample_json).expect("write sample file");

    let out = Command::new(env!("CARGO_BIN_EXE_omnicompress"))
        .arg("eval")
        .arg(&tmp_dir)
        .output()
        .expect("run omnicompress binary");

    assert!(
        out.status.success(),
        "binary exited with non-zero status: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ratio"),
        "stdout missing ratio: {stdout}"
    );
    assert!(
        stdout.contains("roundtrip_ok"),
        "stdout missing roundtrip_ok: {stdout}"
    );

    // Clean up
    let _ = std::fs::remove_file(&sample_path);
    let _ = std::fs::remove_dir(&tmp_dir);
}

/// TDD test: eval must accept real-world **lowercase** role names ("user", "assistant",
/// "system", "tool") — the convention used by the Python binding and by actual session JSON.
///
/// Before the fix this returned `files: []` with a vacuous `roundtrip_ok: true`
/// because the serde PascalCase deserialisation failed and files were silently skipped.
#[test]
fn cli_eval_lowercase_roles_produce_real_ratio() {
    use std::process::Command;

    // Big JSON content to exceed compression threshold (>600 chars, >=20 items).
    let big_json_content = "[".to_string()
        + &(0..60)
            .map(|i| format!(r#"{{"id":{i},"name":"item-{i}","score":{i}.0,"desc":"padding-{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
        + "]";

    // Use lowercase roles as real session JSON would look like.
    let mut blocks = vec![
        msg_block("user", &big_json_content, Some("search")),
    ];
    for i in 0..6 {
        blocks.push(msg_block("assistant", &format!("ok {i}"), None));
    }
    let sample_json = format!("[{}]", blocks.join(","));

    let tmp_dir = std::env::temp_dir().join("omnicompress_eval_lc_test_dir");
    std::fs::create_dir_all(&tmp_dir).expect("create temp dir");
    let sample_path = tmp_dir.join("session.json");
    std::fs::write(&sample_path, &sample_json).expect("write sample file");

    let out = Command::new(env!("CARGO_BIN_EXE_omnicompress"))
        .arg("eval")
        .arg(&tmp_dir)
        .output()
        .expect("run omnicompress binary");

    assert!(
        out.status.success(),
        "binary exited with non-zero status: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    // Must have exactly 1 processed file (not silently dropped).
    let files = parsed["files"].as_array().expect("files array");
    assert_eq!(
        files.len(),
        1,
        "expected 1 processed file, got {} — silent skip still active?\nstdout: {stdout}",
        files.len()
    );

    // ratio must be a real number in (0, 1) — non-null and < 1 means actual compression.
    let ratio = parsed["aggregate"]["ratio"]
        .as_f64()
        .expect("aggregate.ratio should be a non-null number");
    assert!(
        ratio > 0.0 && ratio < 1.0,
        "expected 0 < ratio < 1, got {ratio}\nstdout: {stdout}"
    );

    // roundtrip must be true.
    assert!(
        parsed["aggregate"]["roundtrip_ok"].as_bool().unwrap_or(false),
        "roundtrip_ok should be true\nstdout: {stdout}"
    );

    // Clean up
    let _ = std::fs::remove_file(&sample_path);
    let _ = std::fs::remove_dir(&tmp_dir);
}

/// TDD test: a malformed JSON file must NOT be silently skipped.
/// It must appear in an `errors` array with a `reason`, and be counted.
///
/// Before the fix, parse errors were silently dropped → files:[] with vacuous pass.
#[test]
fn cli_eval_malformed_file_appears_in_errors_not_silently_dropped() {
    use std::process::Command;

    let tmp_dir = std::env::temp_dir().join("omnicompress_eval_malformed_test_dir");
    std::fs::create_dir_all(&tmp_dir).expect("create temp dir");
    let bad_path = tmp_dir.join("bad.json");
    std::fs::write(&bad_path, b"this is not valid json {{{").expect("write bad file");

    let out = Command::new(env!("CARGO_BIN_EXE_omnicompress"))
        .arg("eval")
        .arg(&tmp_dir)
        .output()
        .expect("run omnicompress binary");

    assert!(
        out.status.success(),
        "binary exited with non-zero status: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    // Must have an `errors` array with at least one entry.
    let errors = parsed["errors"].as_array().expect(
        &format!("output should have an `errors` array when files fail to parse\nstdout: {stdout}"),
    );
    assert_eq!(
        errors.len(),
        1,
        "expected 1 error entry, got {}\nstdout: {stdout}",
        errors.len()
    );

    // Each error entry must name the file and give a reason.
    let err0 = &errors[0];
    assert!(
        err0["file"].as_str().is_some(),
        "error entry must have a `file` field\nstdout: {stdout}"
    );
    assert!(
        err0["reason"].as_str().is_some(),
        "error entry must have a `reason` field\nstdout: {stdout}"
    );

    // `files` processed should be empty (no valid files).
    let files = parsed["files"].as_array().expect("files array");
    assert_eq!(
        files.len(),
        0,
        "no valid files should appear in files[]\nstdout: {stdout}"
    );

    // Aggregate errored count must reflect the failed file.
    let errored = parsed["aggregate"]["errored"]
        .as_u64()
        .expect("aggregate.errored should be a number");
    assert_eq!(
        errored, 1,
        "aggregate.errored should be 1\nstdout: {stdout}"
    );

    // Clean up
    let _ = std::fs::remove_file(&bad_path);
    let _ = std::fs::remove_dir(&tmp_dir);
}

/// Helper: serialise a Block as a JSON object string (role as-is, for PascalCase compat tests).
fn serde_block(role: &str, content: &str, tool_name: Option<&str>) -> String {
    let escaped_content = content.replace('\\', "\\\\").replace('"', "\\\"");
    match tool_name {
        Some(t) => format!(
            r#"{{"role":"{role}","content":"{escaped_content}","tool_name":"{t}"}}"#
        ),
        None => format!(
            r#"{{"role":"{role}","content":"{escaped_content}","tool_name":null}}"#
        ),
    }
}

/// Helper: serialise a message with lowercase roles (real-world session format).
fn msg_block(role: &str, content: &str, tool_name: Option<&str>) -> String {
    serde_block(role, content, tool_name)
}

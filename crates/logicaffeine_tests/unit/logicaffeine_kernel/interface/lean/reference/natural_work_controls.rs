// RED controls are marked in comments. This module compiles against the pre-fix API.
fn input(
    payload: serde_json::Value,
    doublings: u64,
    extra: &[serde_json::Value],
) -> super::Reference {
    let mut rows = vec![
        serde_json::json!({"meta":{"format":{"version":"3.1.0"},"lean":{"version":"4.14.0"}}}),
        serde_json::json!({"in":1,"str":{"pre":0,"str":"a"}}),
        serde_json::json!({"in":2,"str":{"pre":0,"str":"b"}}),
        serde_json::json!({"ie":0,"sort":0}),
        serde_json::json!({"ie":1,"natVal":payload}),
    ];
    let mut root = 1;
    for ie in 2..2 + doublings {
        rows.push(serde_json::json!({"ie":ie,"app":{"fn":root,"arg":root}}));
        root = ie;
    }
    rows.push(serde_json::json!({"def":{"name":1,"levelParams":[],"type":0,"value":root,"safety":"safe"}}));
    rows.push(serde_json::json!({"def":{"name":2,"levelParams":[],"type":0,"value":root,"safety":"safe"}}));
    rows.extend_from_slice(extra);
    let text: String = rows.iter().map(|row| row.to_string() + "\n").collect();
    super::Reference::read(&text).unwrap()
}
fn resource<T>(result: super::Result<T>) {
    let error = match result {
        Ok(_) => panic!("expected reference literal workload refusal"),
        Err(e) => e,
    };
    assert_eq!(error.reason().code(), super::DiagnosticCode::ResourceLimit);
}

#[test]
fn red_significant_digits_are_refused_before_large_decimal_conversion() {
    // 3,000 digits cost 3,000 + 2*3,000*188 = 1,131,000 units.
    let reading = input(serde_json::json!("9".repeat(3000)), 0, &[]);
    resource(reading.value("a"));
}

#[test]
fn red_repeated_dag_literal_occurrences_share_one_conversion_allowance() {
    // One 500-digit literal is admitted. Its 32 unfolded uses cost 1,040,000 units.
    let reading = input(serde_json::json!("9".repeat(500)), 5, &[]);
    resource(reading.value("a"));
}

#[test]
fn red_all_zero_payload_bytes_are_admitted_before_scanning() {
    resource(input(serde_json::json!("0".repeat(1_000_001)), 0, &[]).value("a"));
}

#[test]
fn red_repeated_leading_zero_payloads_exhaust_cumulative_byte_work() {
    // Numeric value stays zero; 64 occurrences still scan 1,280,000 input bytes.
    resource(input(serde_json::json!("0".repeat(20_000)), 6, &[]).value("a"));
}

#[test]
fn red_oversized_malformed_payload_is_resource_refused_before_validation() {
    // Pre-fix reader instead scans the whole payload and returns MalformedExport.
    resource(input(serde_json::json!("0".repeat(1_000_000) + "!"), 0, &[]).value("a"));
}

#[test]
fn red_source_preflight_accumulates_literal_work_across_requested_roots() {
    // Each body has 16 uses and is individually below the workload ceiling.
    let reading = input(serde_json::json!("9".repeat(500)), 4, &[]);
    reading.preflight(&[("a".into(), true)], 100, 1000).unwrap();
    resource(reading.preflight(&[("a".into(), true), ("b".into(), true)], 100, 1000));
}

#[test]
fn red_literal_work_is_admitted_before_any_projection_callback() {
    struct Counter(usize);
    impl super::ReferenceProjection for Counter {
        fn project(
            &mut self,
            _: &str,
            _: usize,
            discriminant: super::Term,
            _: &[super::ReferenceBinding<'_>],
            _: &mut usize,
        ) -> super::Result<super::Term> {
            self.0 += 1;
            Ok(discriminant)
        }
    }
    let reading = input(
        serde_json::json!("9".repeat(3000)),
        0,
        &[
            serde_json::json!({"in":3,"str":{"pre":0,"str":"probe"}}),
            serde_json::json!({"ie":2,"proj":{"typeName":1,"idx":0,"struct":0}}),
            serde_json::json!({"ie":3,"app":{"fn":2,"arg":1}}),
            serde_json::json!({"def":{"name":3,"levelParams":[],"type":0,"value":3,"safety":"safe"}}),
        ],
    );
    let mut counter = Counter(0);
    resource(reading.value_with("probe", &mut counter));
    assert_eq!(counter.0, 0);
}

#[test]
fn admitted_values_keep_big_naturals_and_leading_zero_semantics() {
    for digits in [
        "0".into(),
        "000000".into(),
        "18446744073709551616".into(),
        "340282366920938463463374607431768211456".into(),
        "1234567890".repeat(100),
        "0".repeat(2000) + "18446744073709551616",
    ] {
        let value = input(serde_json::json!(&digits), 0, &[])
            .value("a")
            .unwrap();
        let super::Term::Lit(super::Literal::Nat(value)) = value else {
            panic!("compact Nat expected")
        };
        let expected = digits.trim_start_matches('0');
        assert_eq!(
            value.to_string(),
            if expected.is_empty() { "0" } else { expected }
        );
    }
}

#[test]
fn admitted_malformed_payloads_remain_malformed() {
    for payload in [
        serde_json::json!(""),
        serde_json::json!("-1"),
        serde_json::json!("+1"),
        serde_json::json!(" 1"),
        serde_json::json!("1 "),
        serde_json::json!("１"),
        serde_json::json!("1_000"),
        serde_json::json!("1.0"),
        serde_json::json!("1e3"),
        serde_json::json!(5),
        serde_json::json!(null),
    ] {
        let error = input(payload, 0, &[]).value("a").unwrap_err();
        assert_eq!(
            error.reason().code(),
            super::DiagnosticCode::MalformedExport
        );
    }
}

#[test]
fn decimal_work_boundary_is_inclusive_and_independent_of_value_size() {
    // 999,998 input bytes plus two units for the single significant digit.
    let exact = input(serde_json::json!("0".repeat(999_997) + "1"), 0, &[])
        .value("a")
        .unwrap();
    let super::Term::Lit(super::Literal::Nat(one)) = exact else {
        panic!("Nat expected")
    };
    assert_eq!(one.to_string(), "1");
    resource(input(serde_json::json!("0".repeat(999_998) + "1"), 0, &[]).value("a"));
}

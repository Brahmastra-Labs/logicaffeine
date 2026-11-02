use logicaffeine_kernel::interface::lean as adapter;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Input(std::path::PathBuf);
impl Input {
    fn new(accepted: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "logos-stage-trace-{}-{}.ndjson",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let rows = [
            json!({"meta":{"format":{"version":"3.1.0"}}}),
            json!({"in":1,"str":{"pre":0,"str":"diagnostic_subject"}}),
            json!({"il":1,"succ":0}),
            json!({"ie":0,"sort":0}),
            json!({"ie":1,"sort":1}),
            json!({"def":{"all":[1],"name":1,"levelParams":[],
                "type":if accepted {1}else{0},"value":0,"safety":"safe","hints":"opaque"}}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(|r| r.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        Self(path)
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn rows(stderr: &[u8]) -> Vec<Value> {
    std::str::from_utf8(stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("every trace line must be valid JSON"))
        .collect()
}
fn stages(events: &[Value]) -> Vec<(&str, &str)> {
    events
        .iter()
        .filter(|e| e["event"] != "verdict")
        .map(|e| {
            assert_eq!(e["schema"], "logos-lean-stages/1");
            assert_eq!(e["line"], 6);
            assert_eq!(e["name"], "/s:\"diagnostic_subject\"");
            assert_eq!(e["kind"], "def");
            assert_eq!(
                e.as_object().unwrap().len(),
                7,
                "only diagnostic identifiers, never proof terms"
            );
            (e["stage"].as_str().unwrap(), e["event"].as_str().unwrap())
        })
        .collect()
}
#[test]
fn default_cli_keeps_success_silent_and_rejection_behavior_unchanged() {
    for accepted in [true, false] {
        let input = Input::new(accepted);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = adapter::run_cli([input.0.as_os_str().to_owned()], &mut out, &mut err);
        assert_eq!(code, if accepted { 0 } else { 1 });
        if accepted {
            assert_eq!(out, b"ACCEPT\n");
            assert!(err.is_empty());
        } else {
            assert!(out.is_empty());
            assert!(String::from_utf8(err).unwrap().starts_with("Reject("));
        }
    }
}
#[test]
fn opt_in_success_reports_every_stage_in_order_as_json() {
    let input = Input::new(true);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        adapter::run_cli(
            [
                OsString::from("--trace-stages"),
                input.0.as_os_str().to_owned()
            ],
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(out, b"ACCEPT\n");
    let events = rows(&err);
    assert_eq!(
        stages(&events),
        vec![
            ("declaration", "begin"),
            ("type.parse", "begin"),
            ("type.parse", "done"),
            ("type.infer", "begin"),
            ("type.infer", "done"),
            ("body.parse", "begin"),
            ("body.parse", "done"),
            ("body.infer", "begin"),
            ("body.infer", "done"),
            ("exact", "begin"),
            ("exact", "done"),
            ("registration", "begin"),
            ("registration", "done"),
            ("declaration", "done"),
        ]
    );
    assert_eq!(
        events.last().unwrap(),
        &json!({"schema":"logos-lean-stages/1",
        "event":"verdict","code":0,"outcome":"accept"})
    );
}
#[test]
fn opt_in_rejection_identifies_the_unfinished_stage_without_registering() {
    let input = Input::new(false);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        adapter::run_cli(
            [
                input.0.as_os_str().to_owned(),
                OsString::from("--trace-stages")
            ],
            &mut out,
            &mut err
        ),
        1
    );
    assert!(out.is_empty());
    let events = rows(&err);
    assert_eq!(
        stages(&events),
        vec![
            ("declaration", "begin"),
            ("type.parse", "begin"),
            ("type.parse", "done"),
            ("type.infer", "begin"),
            ("type.infer", "done"),
            ("body.parse", "begin"),
            ("body.parse", "done"),
            ("body.infer", "begin"),
            ("body.infer", "done"),
            ("exact", "begin"),
            ("declaration", "error"),
        ]
    );
    assert_eq!(
        events.last().unwrap(),
        &json!({"schema":"logos-lean-stages/1",
        "event":"verdict","code":1,"outcome":"reject"})
    );
}
#[test]
fn unavailable_diagnostic_output_does_not_change_the_checker_verdict() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }
    let input = Input::new(true);
    let mut out = Vec::new();
    assert_eq!(
        adapter::run_cli(
            [
                OsString::from("--trace-stages"),
                input.0.as_os_str().to_owned()
            ],
            &mut out,
            &mut Broken
        ),
        0
    );
    assert_eq!(out, b"ACCEPT\n");
}

#[test]
fn explicit_declaration_audit_runs_between_conversion_and_registration() {
    let input = Input::new(true);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        adapter::run_cli(
            [
                OsString::from("--audit-declarations"),
                input.0.as_os_str().to_owned(),
                OsString::from("--trace-stages"),
            ],
            &mut out,
            &mut err
        ),
        0
    );
    assert_eq!(out, b"ACCEPT\n");
    let events = rows(&err);
    assert_eq!(
        stages(&events),
        vec![
            ("declaration", "begin"),
            ("type.parse", "begin"),
            ("type.parse", "done"),
            ("type.infer", "begin"),
            ("type.infer", "done"),
            ("body.parse", "begin"),
            ("body.parse", "done"),
            ("body.infer", "begin"),
            ("body.infer", "done"),
            ("exact", "begin"),
            ("exact", "done"),
            ("audit", "begin"),
            ("audit", "done"),
            ("registration", "begin"),
            ("registration", "done"),
            ("declaration", "done"),
        ]
    );
    assert_eq!(
        events.last().unwrap(),
        &json!({"schema":"logos-lean-stages/1",
        "event":"verdict","code":0,"outcome":"accept"})
    );
}

#[test]
fn declaration_audit_without_tracing_preserves_cli_output_in_either_argument_order() {
    let input = Input::new(true);
    for arguments in [
        [
            OsString::from("--audit-declarations"),
            input.0.as_os_str().to_owned(),
        ],
        [
            input.0.as_os_str().to_owned(),
            OsString::from("--audit-declarations"),
        ],
    ] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(adapter::run_cli(arguments, &mut out, &mut err), 0);
        assert_eq!(out, b"ACCEPT\n");
        assert!(err.is_empty());
    }
}

#[test]
fn audited_cli_rejects_invalid_source_before_registration_or_audit() {
    let input = Input::new(false);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        adapter::run_cli(
            [
                OsString::from("--trace-stages"),
                OsString::from("--audit-declarations"),
                input.0.as_os_str().to_owned(),
            ],
            &mut out,
            &mut err
        ),
        1
    );
    assert!(out.is_empty());
    let events = rows(&err);
    let actual = stages(&events);
    assert_eq!(actual.last(), Some(&("declaration", "error")));
    assert!(actual.contains(&("exact", "begin")));
    assert!(!actual
        .iter()
        .any(|(stage, _)| matches!(*stage, "audit" | "registration")));
    assert_eq!(
        events.last().unwrap(),
        &json!({"schema":"logos-lean-stages/1",
        "event":"verdict","code":1,"outcome":"reject"})
    );
}

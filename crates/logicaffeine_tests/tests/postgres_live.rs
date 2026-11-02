//! ════════════════════════════════════════════════════════════════════════════════════════════
//! A3 — POSTGRES LIVE SUITE: the pure-LOGOS postgres.lg client against a REAL PostgreSQL 16,
//! downloaded and booted by `postgresql_embedded`. Everything the scripted suite pins against
//! fixtures is proven here against the genuine article: cleartext auth (the embedded server's
//! initdb `--auth password` default), REAL SCRAM-SHA-256 and MD5 (roles + pg_hba lines created
//! through our own client, config reloaded via pg_reload_conf), typed getters over live rows,
//! a real ErrorResponse with the server's SQLSTATE, and rows-affected from real DML.
//!
//! Every test is `#[ignore]`d: the first run downloads a full PostgreSQL distribution.
//! Run deliberately with:
//!     cargo test -p logicaffeine-tests --test postgres_live -- --ignored
//! ════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::time::Duration;

use logicaffeine_compile::{interpret_for_ui, run_vm_io_async};
use postgresql_embedded::{PostgreSQL, Settings, V16};

const SUPER_PASSWORD: &str = "livepw";

/// Download (cached), init, and start a fresh PostgreSQL 16 on a random port.
async fn boot() -> PostgreSQL {
    let mut settings = Settings::new();
    settings.version = V16.clone();
    settings.temporary = true;
    settings.password = SUPER_PASSWORD.to_string();
    let mut pg = PostgreSQL::new(settings);
    pg.setup()
        .await
        .expect("embedded postgres downloads and installs");
    pg.start().await.expect("embedded postgres starts");
    pg
}

fn url_of(pg: &PostgreSQL, user: &str, password: &str, db: &str) -> String {
    let s = pg.settings();
    format!("postgres://{user}:{password}@{}:{}/{db}", s.host, s.port)
}

async fn run_live(program: &str, use_vm: bool) -> Vec<String> {
    let fut = async {
        if use_vm {
            run_vm_io_async(program).await
        } else {
            interpret_for_ui(program).await
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(600), fut)
        .await
        .expect("the live program must not hang");
    assert!(
        result.error.is_none(),
        "live program errored (vm={use_vm}): {:?}",
        result.error
    );
    result.lines
}

/// Run a login program, absorbing TRANSIENT socket-level drops (a fresh
/// postmaster occasionally closes a connection racing a config reload) with a
/// short retry. An AUTH-level rejection (the server's ErrorResponse) is NEVER
/// retried — a wrong password must stay refused.
async fn run_live_retry_transient(program: &str, use_vm: bool) -> Vec<String> {
    for _attempt in 0..3 {
        let lines = run_live(program, use_vm).await;
        let transient = lines.len() == 1 && lines[0].starts_with("refused: pgConnect: tcp:");
        if !transient {
            return lines;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    run_live(program, use_vm).await
}

/// Prepend pg_hba rules (first-match wins, so ours must come before initdb's
/// catch-all `password` lines) and reload the config through OUR OWN client.
async fn prepend_hba_and_reload(pg: &PostgreSQL, rules: &str) {
    let hba_path = pg.settings().data_dir.join("pg_hba.conf");
    let existing = std::fs::read_to_string(&hba_path).expect("pg_hba.conf readable");
    std::fs::write(&hba_path, format!("{rules}\n{existing}")).expect("pg_hba.conf writable");

    let url = url_of(pg, "postgres", SUPER_PASSWORD, "postgres");
    let program = format!(
        "## Main\n\
         Let cR be pgConnect(\"{url}\").\n\
         Inspect cR:\n\
         \x20   When Ok (pgc):\n\
         \x20       Let qR be pgQuery(pgc, \"SELECT pg_reload_conf()\").\n\
         \x20       Inspect qR:\n\
         \x20           When Ok (qr):\n\
         \x20               Let v be pgTextAt(qr, 1, 1).\n\
         \x20               Inspect v:\n\
         \x20                   When Ok (t):\n\
         \x20                       Show t.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be pgClose(pgc).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let lines = run_live(&program, false).await;
    assert_eq!(lines, vec!["t", "done"], "pg_reload_conf must confirm");
}

/// Create a login role with the given password under the given
/// password_encryption, through OUR OWN client on the superuser connection.
async fn create_role(pg: &PostgreSQL, encryption: &str, role: &str, password: &str) {
    let url = url_of(pg, "postgres", SUPER_PASSWORD, "postgres");
    let program = format!(
        "## Main\n\
         Let cR be pgConnect(\"{url}\").\n\
         Inspect cR:\n\
         \x20   When Ok (pgc):\n\
         \x20       Let e1 be pgExecute(pgc, \"SET password_encryption = '{encryption}'\").\n\
         \x20       Inspect e1:\n\
         \x20           When Ok (n):\n\
         \x20               Show n.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let e2 be pgExecute(pgc, \"CREATE ROLE {role} LOGIN PASSWORD '{password}'\").\n\
         \x20       Inspect e2:\n\
         \x20           When Ok (n):\n\
         \x20               Show n.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be pgClose(pgc).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    let lines = run_live(&program, false).await;
    assert_eq!(lines, vec!["0", "0", "done"], "role creation must succeed");
}

/// The end-to-end data path against real PostgreSQL, on BOTH tiers (each tier
/// gets its own database): CREATE TABLE, multi-row INSERT (rows-affected 3),
/// SELECT with ORDER BY — column names, real type OIDs (int4=23, text=25,
/// bool=16, float8=701), typed getters over Int/Text/Bool/Real, NULL as "",
/// wrong-type Err — then UPDATE (2) and DELETE (1), and a clean close.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_cleartext_roundtrip_and_typed_getters() {
    let pg = boot().await;
    for (use_vm, db) in [(false, "live_tw"), (true, "live_vm")] {
        pg.create_database(db).await.expect("create database");
        let url = url_of(&pg, "postgres", SUPER_PASSWORD, db);
        let program = format!(
            "## Main\n\
             Let cR be pgConnect(\"{url}\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let e1 be pgExecute(pgc, \"CREATE TABLE people (id int, name text, active bool, score float8)\").\n\
             \x20       Inspect e1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let e2 be pgExecute(pgc, \"INSERT INTO people VALUES (1, 'alice', true, 2.5), (2, 'bob', false, -0.25), (3, NULL, NULL, NULL)\").\n\
             \x20       Inspect e2:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT id, name, active, score FROM people ORDER BY id\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let colsSeq be qr's columns.\n\
             \x20               Let colsText be join(colsSeq, \",\").\n\
             \x20               Show colsText.\n\
             \x20               Show qr's typeOids.\n\
             \x20               Let rws be qr's rows.\n\
             \x20               Let nrows be length of rws.\n\
             \x20               Show nrows.\n\
             \x20               Let g1 be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect g1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g2 be pgTextAt(qr, 1, 2).\n\
             \x20               Inspect g2:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g3 be pgBoolAt(qr, 1, 3).\n\
             \x20               Inspect g3:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g4 be pgRealAt(qr, 1, 4).\n\
             \x20               Inspect g4:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g5 be pgRealAt(qr, 2, 4).\n\
             \x20               Inspect g5:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g6 be pgTextAt(qr, 3, 2).\n\
             \x20               Inspect g6:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"<{{v}}>\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g7 be pgBoolAt(qr, 3, 3).\n\
             \x20               Inspect g7:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"unexpectedly a Bool\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let e3 be pgExecute(pgc, \"UPDATE people SET active = true WHERE id <= 2\").\n\
             \x20       Inspect e3:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let e4 be pgExecute(pgc, \"DELETE FROM people WHERE id = 3\").\n\
             \x20       Inspect e4:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Inspect closed:\n\
             \x20           When Ok (u):\n\
             \x20               Show \"closed\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        );
        let lines = run_live(&program, use_vm).await;
        assert_eq!(
            lines,
            vec![
                "0",
                "3",
                "id,name,active,score",
                "[23, 25, 16, 701]",
                "3",
                "1",
                "alice",
                "true",
                "2.5",
                "-0.25",
                "<>",
                "pg: the value '' is not a Bool",
                "2",
                "1",
                "closed",
            ],
            "tier vm={use_vm} diverged against real PostgreSQL"
        );
    }
    pg.stop().await.ok();
}

/// A REAL ErrorResponse: the server's own SQLSTATE 42601 arrives as an Err
/// value, and the connection stays usable for the next query.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_error_response_carries_the_real_sqlstate() {
    let pg = boot().await;
    let url = url_of(&pg, "postgres", SUPER_PASSWORD, "postgres");
    let program = format!(
        "## Main\n\
         Let cR be pgConnect(\"{url}\").\n\
         Inspect cR:\n\
         \x20   When Ok (pgc):\n\
         \x20       Let qR be pgQuery(pgc, \"SELECT * FROM\").\n\
         \x20       Inspect qR:\n\
         \x20           When Ok (qr):\n\
         \x20               Show \"unexpectedly ok\".\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let q2 be pgQuery(pgc, \"SELECT 41 + 1\").\n\
         \x20       Inspect q2:\n\
         \x20           When Ok (qr):\n\
         \x20               Let v be pgIntAt(qr, 1, 1).\n\
         \x20               Inspect v:\n\
         \x20                   When Ok (n):\n\
         \x20                       Show n.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be pgClose(pgc).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    for use_vm in [false, true] {
        let lines = run_live(&program, use_vm).await;
        assert_eq!(lines.len(), 3, "vm={use_vm}: {lines:?}");
        assert!(
            lines[0].starts_with("ERROR 42601: syntax error"),
            "vm={use_vm}: expected the real syntax-error SQLSTATE, got {:?}",
            lines[0]
        );
        assert_eq!(
            lines[1], "42",
            "vm={use_vm}: the connection must stay usable"
        );
        assert_eq!(lines[2], "done");
    }
    pg.stop().await.ok();
}

/// FULL SCRAM-SHA-256 against real PostgreSQL: a role stored with a
/// scram-sha-256 verifier, a pg_hba rule demanding scram, and our client's
/// complete RFC 5802 exchange (i=4096, the server's real salt and nonce) —
/// then a query as that role. The wrong password must be REJECTED as an Err.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_scram_sha256_auth() {
    let pg = boot().await;
    create_role(&pg, "scram-sha-256", "scramuser", "scr4m pw").await;
    prepend_hba_and_reload(
        &pg,
        "host all scramuser 127.0.0.1/32 scram-sha-256\nhost all scramuser ::1/128 scram-sha-256",
    )
    .await;

    let good = |url: String| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"{url}\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT current_user\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v be pgTextAt(qr, 1, 1).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (t):\n\
             \x20                       Show t.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show \"refused: {{e}}\".\n"
        )
    };

    for use_vm in [false, true] {
        let url = url_of(&pg, "scramuser", "scr4m pw", "postgres");
        let lines = run_live_retry_transient(&good(url), use_vm).await;
        let server_log = std::fs::read_to_string(pg.settings().data_dir.join("start.log"))
            .unwrap_or_else(|_| "<no start.log>".into());
        assert_eq!(
            lines,
            vec!["scramuser", "done"],
            "vm={use_vm}: real SCRAM login failed\n--- server log ---\n{}",
            server_log
                .lines()
                .rev()
                .take(30)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    // The wrong password is an Err value, on both tiers.
    for use_vm in [false, true] {
        let url = url_of(&pg, "scramuser", "wrongpw", "postgres");
        let lines = run_live(&good(url), use_vm).await;
        assert_eq!(lines.len(), 1, "vm={use_vm}: {lines:?}");
        assert!(
            lines[0].starts_with("refused: "),
            "vm={use_vm}: a wrong SCRAM password must be refused, got {:?}",
            lines[0]
        );
    }
    pg.stop().await.ok();
}

/// REAL MD5 auth: a role stored with an md5 verifier and an md5 pg_hba rule —
/// the server sends AuthenticationMD5Password with its 4 random salt bytes and
/// our double-hash construction must satisfy it.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_md5_auth() {
    let pg = boot().await;
    create_role(&pg, "md5", "md5user", "md5 pw").await;
    prepend_hba_and_reload(
        &pg,
        "host all md5user 127.0.0.1/32 md5\nhost all md5user ::1/128 md5",
    )
    .await;

    let program = |url: String| {
        format!(
            "## Main\n\
             Let cR be pgConnect(\"{url}\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let qR be pgQuery(pgc, \"SELECT current_user\").\n\
             \x20       Inspect qR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v be pgTextAt(qr, 1, 1).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (t):\n\
             \x20                       Show t.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show \"refused: {{e}}\".\n"
        )
    };

    for use_vm in [false, true] {
        let url = url_of(&pg, "md5user", "md5 pw", "postgres");
        let lines = run_live_retry_transient(&program(url), use_vm).await;
        assert_eq!(
            lines,
            vec!["md5user", "done"],
            "vm={use_vm}: real MD5 login failed"
        );
    }

    for use_vm in [false, true] {
        let url = url_of(&pg, "md5user", "nope", "postgres");
        let lines = run_live(&program(url), use_vm).await;
        assert_eq!(lines.len(), 1, "vm={use_vm}: {lines:?}");
        assert!(
            lines[0].starts_with("refused: "),
            "vm={use_vm}: a wrong MD5 password must be refused, got {:?}",
            lines[0]
        );
    }
    pg.stop().await.ok();
}

// ═══════════════════════════════════════════════════════════════════════════
// G3 — the EXTENDED QUERY PROTOCOL and typed values against the genuine
// article: prepared statements with params (incl. NULL both directions),
// exact numerics, uuid, enums, transactions (rollback + a REAL serializable
// conflict, SQLSTATE 40001), statement_timeout (SQLSTATE 57014), and the
// cancel packet against a REAL postmaster.
// ═══════════════════════════════════════════════════════════════════════════

/// Prepared statements are THE parameterized path: prepare captures the
/// declared param OIDs and result shape; execute binds text params — with a
/// NULL param via the mask — and a NULL result cell is mask-true/Opt-none
/// while a present EMPTY STRING stays distinct. DML through the prepared
/// path reports rows-affected via the cmdTag.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_prepared_roundtrip_with_params_including_null() {
    let pg = boot().await;
    for (use_vm, db) in [(false, "prep_tw"), (true, "prep_vm")] {
        pg.create_database(db).await.expect("create database");
        let url = url_of(&pg, "postgres", SUPER_PASSWORD, db);
        let program = format!(
            "## Main\n\
             Let cR be pgConnect(\"{url}\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let e1 be pgExecute(pgc, \"CREATE TABLE items (id int, label text)\").\n\
             \x20       Inspect e1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let pIns be pgPrepare(pgc, \"ins\", \"INSERT INTO items VALUES ($1, $2)\").\n\
             \x20       Inspect pIns:\n\
             \x20           When Ok (prep):\n\
             \x20               Show prep's paramOids.\n\
             \x20               Show prep's hasRows.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let ps1: Seq of Text be [\"1\", \"alpha\"].\n\
             \x20       Let x1 be pgExecPrepared(pgc, \"ins\", ps1).\n\
             \x20       Inspect x1:\n\
             \x20           When Ok (qr):\n\
             \x20               Let tag1 be qr's cmdTag.\n\
             \x20               Show pgRowsAffected(tag1).\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let ps2: Seq of Text be [\"2\", \"\"].\n\
             \x20       Let ns2: Seq of Bool be [false, true].\n\
             \x20       Let x2 be pgExecPreparedN(pgc, \"ins\", ps2, ns2).\n\
             \x20       Inspect x2:\n\
             \x20           When Ok (qr):\n\
             \x20               Let tag2 be qr's cmdTag.\n\
             \x20               Show pgRowsAffected(tag2).\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let ps3: Seq of Text be [\"3\", \"\"].\n\
             \x20       Let x3 be pgExecPrepared(pgc, \"ins\", ps3).\n\
             \x20       Inspect x3:\n\
             \x20           When Ok (qr):\n\
             \x20               Let tag3 be qr's cmdTag.\n\
             \x20               Show pgRowsAffected(tag3).\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let pSel be pgPrepare(pgc, \"sel\", \"SELECT id, label FROM items ORDER BY id\").\n\
             \x20       Inspect pSel:\n\
             \x20           When Ok (prep):\n\
             \x20               Show prep's paramOids.\n\
             \x20               Show prep's hasRows.\n\
             \x20               Let colsSeq be prep's columns.\n\
             \x20               Let colsText be join(colsSeq, \",\").\n\
             \x20               Show colsText.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let noPs be a new Seq of Text.\n\
             \x20       Let sR be pgExecPrepared(pgc, \"sel\", noPs).\n\
             \x20       Inspect sR:\n\
             \x20           When Ok (qr):\n\
             \x20               Let rws be qr's rows.\n\
             \x20               Show length of rws.\n\
             \x20               Let g1 be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect g1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g2 be pgTextAtOpt(qr, 1, 2).\n\
             \x20               Inspect g2:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g3 be pgTextAtOpt(qr, 2, 2).\n\
             \x20               Inspect g3:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g4 be pgIsNullAt(qr, 2, 2).\n\
             \x20               Inspect g4:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g5 be pgTextAt(qr, 2, 2).\n\
             \x20               Inspect g5:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"<{{v}}>\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g6 be pgTextAtOpt(qr, 3, 2).\n\
             \x20               Inspect g6:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let g7 be pgIsNullAt(qr, 3, 2).\n\
             \x20               Inspect g7:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let qpArgs: Seq of Text be [\"2\"].\n\
             \x20       Let qp be pgQueryParams(pgc, \"SELECT label FROM items WHERE id = $1\", qpArgs).\n\
             \x20       Inspect qp:\n\
             \x20           When Ok (qr):\n\
             \x20               Let o be pgTextAtOpt(qr, 1, 1).\n\
             \x20               Inspect o:\n\
             \x20                   When Ok (ov):\n\
             \x20                       Inspect ov:\n\
             \x20                           When Some (v):\n\
             \x20                               Show \"some <{{v}}>\".\n\
             \x20                           When None:\n\
             \x20                               Show \"none\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        );
        let lines = run_live(&program, use_vm).await;
        assert_eq!(
            lines,
            vec![
                "0",
                "[23, 25]",
                "false",
                "1",
                "1",
                "1",
                "[]",
                "true",
                "id,label",
                "3",
                "1",
                "some <alpha>",
                "none",
                "true",
                "<>",
                "some <>",
                "false",
                "none",
                "done",
            ],
            "tier vm={use_vm} diverged on the live prepared round-trip"
        );
    }
    pg.stop().await.ok();
}

/// Typed mappings against live values: numeric survives EXACTLY at 30+30
/// digits through pgDecimalAt, uuid parses + normalizes, enum labels check
/// membership (and reject an unexpected set), and pgTypedAt validates by the
/// REAL RowDescription OIDs.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_decimal_uuid_and_enum_typed_mappings() {
    let big = "123456789012345678901234567890.123456789012345678901234567890";
    let pg = boot().await;
    for (use_vm, db) in [(false, "typed_tw"), (true, "typed_vm")] {
        pg.create_database(db).await.expect("create database");
        let url = url_of(&pg, "postgres", SUPER_PASSWORD, db);
        let program = format!(
            "## Main\n\
             Let cR be pgConnect(\"{url}\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let e0 be pgExecute(pgc, \"CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')\").\n\
             \x20       Inspect e0:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let e1 be pgExecute(pgc, \"CREATE TABLE vals (n numeric, u uuid, m mood)\").\n\
             \x20       Inspect e1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let ps: Seq of Text be [\"{big}\", \"A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11\", \"happy\"].\n\
             \x20       Let ins be pgQueryParams(pgc, \"INSERT INTO vals VALUES ($1::numeric, $2::uuid, $3::mood)\", ps).\n\
             \x20       Inspect ins:\n\
             \x20           When Ok (qr):\n\
             \x20               Let tagT be qr's cmdTag.\n\
             \x20               Show pgRowsAffected(tagT).\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let sel be pgQuery(pgc, \"SELECT n, u, m FROM vals\").\n\
             \x20       Inspect sel:\n\
             \x20           When Ok (qr):\n\
             \x20               Show qr's typeOids.\n\
             \x20               Let d be pgDecimalAt(qr, 1, 1).\n\
             \x20               Inspect d:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let u be pgUuidAt(qr, 1, 2).\n\
             \x20               Inspect u:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show uuidFormat(v).\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let allowed: Seq of Text be [\"sad\", \"ok\", \"happy\"].\n\
             \x20               Let m1 be pgEnumAt(qr, 1, 3, allowed).\n\
             \x20               Inspect m1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let wrongSet: Seq of Text be [\"red\", \"green\"].\n\
             \x20               Let m2 be pgEnumAt(qr, 1, 3, wrongSet).\n\
             \x20               Inspect m2:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show \"unexpectedly allowed\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let t1 be pgTypedAt(qr, 1, 1).\n\
             \x20               Inspect t1:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let t2 be pgTypedAt(qr, 1, 2).\n\
             \x20               Inspect t2:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let t3 be pgTypedAt(qr, 1, 3).\n\
             \x20               Inspect t3:\n\
             \x20                   When Ok (v):\n\
             \x20                       Show v.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        );
        let lines = run_live(&program, use_vm).await;
        assert_eq!(lines.len(), 12, "vm={use_vm}: {lines:?}");
        assert_eq!(lines[0], "0", "vm={use_vm}: CREATE TYPE");
        assert_eq!(lines[1], "0", "vm={use_vm}: CREATE TABLE");
        assert_eq!(
            lines[2], "1",
            "vm={use_vm}: INSERT through the parameterized path"
        );
        assert!(
            lines[3].starts_with("[1700, 2950, "),
            "vm={use_vm}: numeric+uuid OIDs then the enum's fresh OID, got {:?}",
            lines[3]
        );
        assert_eq!(
            lines[4], big,
            "vm={use_vm}: numeric must round-trip EXACTLY"
        );
        assert_eq!(
            lines[5], "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "vm={use_vm}: uuid parses + normalizes"
        );
        assert_eq!(lines[6], "happy", "vm={use_vm}: enum membership");
        assert_eq!(
            lines[7], "pg: the value 'happy' is not one of the expected enum values",
            "vm={use_vm}: enum rejection"
        );
        assert_eq!(lines[8], big, "vm={use_vm}: pgTypedAt numeric");
        assert_eq!(
            lines[9], "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
            "vm={use_vm}: pgTypedAt uuid"
        );
        assert_eq!(
            lines[10], "happy",
            "vm={use_vm}: pgTypedAt enum passthrough"
        );
        assert_eq!(lines[11], "done");
    }
    pg.stop().await.ok();
}

/// Transactions with explicit boundaries: ROLLBACK really discards, and TWO
/// serializable transactions in a genuine write-skew race surface SQLSTATE
/// 40001 as an ordinary Err (never a trap) — after which the loser rolls
/// back and the connection keeps working.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_transactions_rollback_and_serializable_conflict() {
    let pg = boot().await;
    for (use_vm, db) in [(false, "tx_tw"), (true, "tx_vm")] {
        pg.create_database(db).await.expect("create database");
        let url = url_of(&pg, "postgres", SUPER_PASSWORD, db);
        let program = format!(
            "## Main\n\
             Let cR be pgConnect(\"{url}\").\n\
             Inspect cR:\n\
             \x20   When Ok (pgc):\n\
             \x20       Let e1 be pgExecute(pgc, \"CREATE TABLE t (id int primary key, v int)\").\n\
             \x20       Inspect e1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let e2 be pgExecute(pgc, \"INSERT INTO t VALUES (1, 10), (2, 20)\").\n\
             \x20       Inspect e2:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let b1 be pgBegin(pgc).\n\
             \x20       Inspect b1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let i1 be pgExecute(pgc, \"INSERT INTO t VALUES (99, 990)\").\n\
             \x20       Inspect i1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let r1 be pgRollback(pgc).\n\
             \x20       Inspect r1:\n\
             \x20           When Ok (n):\n\
             \x20               Show n.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let c1 be pgQuery(pgc, \"SELECT count(*) FROM t\").\n\
             \x20       Inspect c1:\n\
             \x20           When Ok (qr):\n\
             \x20               Let v be pgIntAt(qr, 1, 1).\n\
             \x20               Inspect v:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let c2R be pgConnect(\"{url}\").\n\
             \x20       Inspect c2R:\n\
             \x20           When Ok (pgc2):\n\
             \x20               Let ba be pgBeginIsolation(pgc, \"serializable\").\n\
             \x20               Inspect ba:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let bb be pgBeginIsolation(pgc2, \"serializable\").\n\
             \x20               Inspect bb:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let sa be pgQuery(pgc, \"SELECT sum(v) FROM t\").\n\
             \x20               Inspect sa:\n\
             \x20                   When Ok (qr):\n\
             \x20                       Let v be pgIntAt(qr, 1, 1).\n\
             \x20                       Inspect v:\n\
             \x20                           When Ok (n):\n\
             \x20                               Show n.\n\
             \x20                           When Err (e):\n\
             \x20                               Show e.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let sb be pgQuery(pgc2, \"SELECT sum(v) FROM t\").\n\
             \x20               Inspect sb:\n\
             \x20                   When Ok (qr):\n\
             \x20                       Let v be pgIntAt(qr, 1, 1).\n\
             \x20                       Inspect v:\n\
             \x20                           When Ok (n):\n\
             \x20                               Show n.\n\
             \x20                           When Err (e):\n\
             \x20                               Show e.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let ia be pgExecute(pgc, \"INSERT INTO t VALUES (3, 30)\").\n\
             \x20               Inspect ia:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show \"errA: {{e}}\".\n\
             \x20               Let ib be pgExecute(pgc2, \"INSERT INTO t VALUES (4, 30)\").\n\
             \x20               Inspect ib:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show n.\n\
             \x20                   When Err (e):\n\
             \x20                       Show \"errB: {{e}}\".\n\
             \x20               Let ca be pgCommit(pgc).\n\
             \x20               Inspect ca:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show \"commitA ok\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show \"errA: {{e}}\".\n\
             \x20               Let cb be pgCommit(pgc2).\n\
             \x20               Inspect cb:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show \"commitB ok\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show \"errB: {{e}}\".\n\
             \x20               Let rb be pgRollback(pgc2).\n\
             \x20               Inspect rb:\n\
             \x20                   When Ok (n):\n\
             \x20                       Show \"rolledB\".\n\
             \x20                   When Err (e):\n\
             \x20                       Show \"errB: {{e}}\".\n\
             \x20               Let cnt be pgQuery(pgc2, \"SELECT count(*) FROM t\").\n\
             \x20               Inspect cnt:\n\
             \x20                   When Ok (qr):\n\
             \x20                       Let v be pgIntAt(qr, 1, 1).\n\
             \x20                       Inspect v:\n\
             \x20                           When Ok (n):\n\
             \x20                               Show n.\n\
             \x20                           When Err (e):\n\
             \x20                               Show e.\n\
             \x20                   When Err (e):\n\
             \x20                       Show e.\n\
             \x20               Let closed2 be pgClose(pgc2).\n\
             \x20               Show \"closed2\".\n\
             \x20           When Err (e):\n\
             \x20               Show e.\n\
             \x20       Let closed be pgClose(pgc).\n\
             \x20       Show \"done\".\n\
             \x20   When Err (e):\n\
             \x20       Show e.\n"
        );
        let lines = run_live(&program, use_vm).await;
        assert!(
            lines.len() >= 15,
            "vm={use_vm}: unexpectedly short output {lines:?}"
        );
        assert_eq!(
            &lines[0..6],
            &["0", "2", "0", "1", "0", "2"],
            "vm={use_vm}: rollback must discard, {lines:?}"
        );
        assert_eq!(
            &lines[6..10],
            &["0", "0", "30", "30"],
            "vm={use_vm}: both serializable txs read 30, {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("40001")),
            "vm={use_vm}: the serializable conflict must surface SQLSTATE 40001 as an Err, {lines:?}"
        );
        assert_eq!(
            lines[lines.len() - 3],
            "3",
            "vm={use_vm}: only the winner's insert persists, {lines:?}"
        );
        assert_eq!(lines[lines.len() - 2], "closed2", "vm={use_vm}: {lines:?}");
        assert_eq!(
            lines[lines.len() - 1],
            "done",
            "vm={use_vm}: no trap after the conflict, {lines:?}"
        );
    }
    pg.stop().await.ok();
}

/// statement_timeout is the deterministic cancellation lock: a pg_sleep
/// that exceeds the timeout dies with SQLSTATE 57014 (query_canceled,
/// errcodes.txt:434) as an ordinary Err, and the session keeps working.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_statement_timeout_surfaces_57014() {
    let pg = boot().await;
    let url = url_of(&pg, "postgres", SUPER_PASSWORD, "postgres");
    let program = format!(
        "## Main\n\
         Let cR be pgConnect(\"{url}\").\n\
         Inspect cR:\n\
         \x20   When Ok (pgc):\n\
         \x20       Let s1 be pgSetStatementTimeout(pgc, 150).\n\
         \x20       Inspect s1:\n\
         \x20           When Ok (n):\n\
         \x20               Show n.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let q1 be pgQuery(pgc, \"SELECT pg_sleep(2)\").\n\
         \x20       Inspect q1:\n\
         \x20           When Ok (qr):\n\
         \x20               Show \"unexpectedly survived\".\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let s2 be pgSetStatementTimeout(pgc, 0).\n\
         \x20       Inspect s2:\n\
         \x20           When Ok (n):\n\
         \x20               Show n.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let q2 be pgQuery(pgc, \"SELECT 41 + 1\").\n\
         \x20       Inspect q2:\n\
         \x20           When Ok (qr):\n\
         \x20               Let v be pgIntAt(qr, 1, 1).\n\
         \x20               Inspect v:\n\
         \x20                   When Ok (n):\n\
         \x20                       Show n.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be pgClose(pgc).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    for use_vm in [false, true] {
        let lines = run_live(&program, use_vm).await;
        assert_eq!(lines.len(), 5, "vm={use_vm}: {lines:?}");
        assert_eq!(lines[0], "0");
        assert!(
            lines[1].starts_with("ERROR 57014:"),
            "vm={use_vm}: expected SQLSTATE 57014 (query_canceled), got {:?}",
            lines[1]
        );
        assert_eq!(lines[2], "0");
        assert_eq!(lines[3], "42", "vm={use_vm}: the session must keep working");
        assert_eq!(lines[4], "done");
    }
    pg.stop().await.ok();
}

/// Best-effort cancel probe against a REAL postmaster: the BackendKeyData is
/// captured, the 16-byte packet is dispatched on a fresh connection (the
/// byte-exactness lock lives in the scripted suite), and the session is
/// unharmed afterwards.
#[tokio::test]
#[ignore = "live: downloads+runs real PostgreSQL"]
async fn live_cancel_request_best_effort() {
    let pg = boot().await;
    let url = url_of(&pg, "postgres", SUPER_PASSWORD, "postgres");
    let program = format!(
        "## Main\n\
         Let cR be pgConnect(\"{url}\").\n\
         Inspect cR:\n\
         \x20   When Ok (pgc):\n\
         \x20       Let pid be pgc's backendPid.\n\
         \x20       If pid is greater than 0:\n\
         \x20           Show \"pid captured\".\n\
         \x20       If pid is at most 0:\n\
         \x20           Show \"pid missing\".\n\
         \x20       Let can be pgCancelRequest(pgc).\n\
         \x20       Inspect can:\n\
         \x20           When Ok (u):\n\
         \x20               Show \"cancel sent\".\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let q be pgQuery(pgc, \"SELECT 41 + 1\").\n\
         \x20       Inspect q:\n\
         \x20           When Ok (qr):\n\
         \x20               Let v be pgIntAt(qr, 1, 1).\n\
         \x20               Inspect v:\n\
         \x20                   When Ok (n):\n\
         \x20                       Show n.\n\
         \x20                   When Err (e):\n\
         \x20                       Show e.\n\
         \x20           When Err (e):\n\
         \x20               Show e.\n\
         \x20       Let closed be pgClose(pgc).\n\
         \x20       Show \"done\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    for use_vm in [false, true] {
        let lines = run_live(&program, use_vm).await;
        assert_eq!(
            lines,
            vec!["pid captured", "cancel sent", "42", "done"],
            "vm={use_vm}: the cancel dispatch must leave the session usable"
        );
    }
    pg.stop().await.ok();
}

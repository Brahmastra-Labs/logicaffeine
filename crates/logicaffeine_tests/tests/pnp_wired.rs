//! `Connect to <addr> with pad "<path>" as initiator` (and `Listen … as responder`) is wired
//! end-to-end: running such a program under the interpreter installs a PNP one-time-pad session on the
//! channel, so every subsequent `Send` is sealed and fail-closed on pad exhaustion. A bare
//! `Connect`/`Listen` installs nothing — byte-identical passthrough, exactly as before.

use futures::executor::block_on;
use logicaffeine_compile::concurrency::{channel, pnp, set_net_offline};
use logicaffeine_compile::interpret_for_ui;

/// Offline single-node mode: `Connect` skips the real relay dial (there is no tokio runtime under
/// `block_on`), but the pad session still installs — that is independent of the transport.
fn offline_slate() {
    set_net_offline(true);
    channel::install_session(None);
}

/// Full-byte-entropy pad material the incompressibility gate accepts.
fn random_pad(len: usize, seed: u64) -> Vec<u8> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) as u8
        })
        .collect()
}

fn write_pad(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("pnp_wired_{}_{}.pad", std::process::id(), tag));
    std::fs::write(&path, random_pad(4096, 0xA1B2_C3D4_E5F6_0718)).expect("write pad file");
    path
}

#[test]
fn connect_with_pad_installs_pnp_session() {
    offline_slate();
    let path = write_pad("connect");
    let src = format!(
        "## Main\nConnect to \"/ip4/127.0.0.1/tcp/9990\" with pad \"{}\" as initiator.\n",
        path.display()
    );
    let result = block_on(interpret_for_ui(&src));
    assert!(result.error.is_none(), "program should run: {:?}", result.error);

    // The session is installed: the exact fn the interpreter Send path calls now seals a PNP cover.
    assert!(channel::active_session().is_some(), "`Connect … with pad` must install a PNP session");
    let frame = channel::seal_active_checked(b"probe".to_vec()).expect("seal succeeds while pad remains");
    assert!(
        pnp::frame_offset(&frame).is_some(),
        "outbound bytes are a PNP frame, got prefix {:?}",
        &frame[..frame.len().min(6)]
    );
    assert_ne!(frame, b"probe".to_vec(), "sealed, not plaintext");

    channel::install_session(None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn listen_with_pad_installs_pnp_session_responder() {
    offline_slate();
    let path = write_pad("listen");
    let src = format!(
        "## Main\nListen on \"/ip4/127.0.0.1/tcp/9991\" with pad \"{}\" as responder.\n",
        path.display()
    );
    let result = block_on(interpret_for_ui(&src));
    assert!(result.error.is_none(), "program should run: {:?}", result.error);
    assert!(channel::active_session().is_some(), "`Listen … with pad` must install a PNP session");

    channel::install_session(None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn bare_connect_installs_no_session() {
    offline_slate();
    let src = "## Main\nConnect to \"/ip4/127.0.0.1/tcp/9992\".\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "program should run: {:?}", result.error);
    assert!(channel::active_session().is_none(), "a bare Connect installs no session (passthrough)");
}

#[test]
fn a_missing_pad_file_fails_the_send_path_closed() {
    // A `with pad` naming an unreadable file must error the statement (fail-closed), never silently
    // fall back to plaintext.
    offline_slate();
    let src = "## Main\nConnect to \"/ip4/127.0.0.1/tcp/9993\" with pad \"/no/such/pnp/pad/file.pad\" as initiator.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_some(), "an unreadable pad must fail the program, not proceed insecurely");
    assert!(channel::active_session().is_none(), "no session installed on failure");
}

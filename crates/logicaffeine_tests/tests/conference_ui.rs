//! Conference UI, authored entirely in Logos — the last frontend primitive.
//!
//! The reactive toolkit (signals, components, `View`, keyed `for each`, `bound to`, scoped CSS,
//! events) already builds the whole conference shell; the one thing it lacked was attaching a live
//! `MediaStream` to a `<video>` tile. `setStream(el, handle)` closes that: a video element is
//! `createElement("video")`, and the peer's opaque stream handle is bound to its `srcObject`. Here
//! we lock the video-tile-with-stream composition and a two-peer gallery, IDENTICAL on the
//! tree-walker and the VM — so a conferencing UI is now fully expressible in Logos.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};

fn interp_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = tw_outcome(src);
    dom::op_log()
}

fn vm_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = vm_outcome(src);
    dom::op_log()
}

fn assert_dom_parity(src: &str, expected: &[DomOp]) {
    let interp = interp_log(src);
    let vm = vm_log(src);
    assert_eq!(interp, vm, "tree-walker vs VM UI op-log diverged:\n{src}");
    assert_eq!(interp, expected, "op-log mismatch:\n{src}");
}

/// A single video tile: a `<video>` element with a peer id, its live stream bound to `srcObject`.
#[test]
fn a_video_tile_binds_a_live_stream() {
    let src = "## Main\n\
        Let tile be createElement(\"video\").\n\
        Call setAttribute with tile and \"id\" and \"peer-1\".\n\
        Call setStream with tile and 7.\n\
        Call appendChild with body() and tile.\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "video".into(),
            },
            DomOp::SetAttribute {
                node: 1,
                name: "id".into(),
                value: "peer-1".into(),
            },
            DomOp::SetStream { node: 1, stream: 7 },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
        ],
    );
}

/// A participant gallery: a grid `div` with one `<video>` tile per peer, each bound to that peer's
/// stream handle — the core conferencing layout, built in Logos, identical across tiers.
#[test]
fn a_conference_gallery_builds_a_video_tile_per_peer() {
    let src = "## Main\n\
        Let gallery be createElement(\"div\").\n\
        Call setAttribute with gallery and \"class\" and \"gallery\".\n\
        Let alice be createElement(\"video\").\n\
        Call setStream with alice and 10.\n\
        Call appendChild with gallery and alice.\n\
        Let bob be createElement(\"video\").\n\
        Call setStream with bob and 20.\n\
        Call appendChild with gallery and bob.\n\
        Call appendChild with body() and gallery.\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "div".into(),
            },
            DomOp::SetAttribute {
                node: 1,
                name: "class".into(),
                value: "gallery".into(),
            },
            DomOp::CreateElement {
                node: 2,
                tag: "video".into(),
            },
            DomOp::SetStream {
                node: 2,
                stream: 10,
            },
            DomOp::AppendChild {
                parent: 1,
                child: 2,
            },
            DomOp::CreateElement {
                node: 3,
                tag: "video".into(),
            },
            DomOp::SetStream {
                node: 3,
                stream: 20,
            },
            DomOp::AppendChild {
                parent: 1,
                child: 3,
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
        ],
    );
}

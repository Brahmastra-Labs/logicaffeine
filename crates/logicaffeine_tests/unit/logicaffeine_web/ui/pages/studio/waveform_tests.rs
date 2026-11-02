use super::waveform_svg;
use logicaffeine_compile::codegen_sva::hw_pipeline::{Waveform, WaveSignal};

fn attr<'a>(svg: &'a str, name: &str) -> Option<&'a str> {
    let head = &svg[..svg.find('>')?];
    let at = head.find(&format!(" {name}=\""))? + name.len() + 3;
    head[at..].split('"').next()
}

/// A trace is data: it draws at its own size and its panel scrolls, so a long trace keeps
/// its labels readable instead of shrinking them to fit.
#[test]
fn a_waveform_draws_at_its_own_size() {
    let wf = Waveform {
        timesteps: 16,
        signals: vec![WaveSignal { name: "q".into(), width: 3, values: (0..16).map(|t| Some(t % 8)).collect() }],
    };
    let svg = waveform_svg(&wf);
    let view: Vec<&str> = attr(&svg, "viewBox").expect("a viewBox").split_whitespace().collect();
    assert_eq!(attr(&svg, "width"), Some(view[2]), "the width is not the drawing's own: {}", &svg[..120]);
    assert_eq!(attr(&svg, "height"), Some(view[3]), "the height is not the drawing's own: {}", &svg[..120]);
}

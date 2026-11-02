use super::{countdown_seq, ns_drive, traffic_plan, traffic_svg, NsDrive};
use logicaffeine_compile::codegen_sva::hw_pipeline::{WaveSignal, Waveform};

fn sig(name: &str, vals: &[u64]) -> WaveSignal {
    WaveSignal {
        name: name.to_string(),
        width: 2,
        values: vals.iter().map(|&v| Some(v)).collect(),
    }
}
fn wf(signals: Vec<WaveSignal>, n: u32) -> Waveform {
    Waveform {
        timesteps: n,
        signals,
    }
}

/// A non-conflicting trace: NS green (2 clocks), NS yellow (1), all-red (1), WALK (2).
fn safe_trace() -> Waveform {
    wf(
        vec![
            sig("ns", &[1, 1, 2, 0, 0, 0]),
            sig("ew", &[0, 0, 0, 0, 0, 0]),
            sig("ped", &[0, 0, 0, 0, 1, 1]),
        ],
        6,
    )
}

#[test]
fn collapses_consecutive_identical_states_into_phases() {
    let plan = traffic_plan(&safe_trace()).unwrap();
    assert_eq!(plan.phases.len(), 4, "ns-green, ns-yellow, all-red, walk");
    assert_eq!(plan.firsts, vec![0, 2, 3, 4]);
    assert_eq!(plan.phases[0].ns, 1);
    assert_eq!(plan.phases[1].ns, 2);
    assert_eq!(plan.phases[3].ped, 1);
}

#[test]
fn phase_durations_are_human_paced_and_cumulative() {
    let plan = traffic_plan(&safe_trace()).unwrap();
    assert_eq!(plan.phases[0].dur, 2.6, "green");
    assert_eq!(plan.phases[1].dur, 1.3, "yellow");
    assert_eq!(plan.phases[2].dur, 1.0, "all-red");
    assert_eq!(plan.phases[3].dur, 7.0, "walk");
    assert_eq!(plan.phases[0].start, 0.0);
    assert_eq!(plan.phases[1].start, 2.6);
    assert!((plan.phases[3].start - (2.6 + 1.3 + 1.0)).abs() < 1e-9);
    assert!((plan.total - (2.6 + 1.3 + 1.0 + 7.0)).abs() < 1e-9);
}

#[test]
fn exclusive_movements_have_no_conflict() {
    assert_eq!(traffic_plan(&safe_trace()).unwrap().conflict, None);
}

#[test]
fn pedestrian_walking_into_green_traffic_is_a_conflict() {
    let plan = traffic_plan(&wf(
        vec![sig("ns", &[1, 1]), sig("ew", &[0, 0]), sig("ped", &[1, 1])],
        2,
    ))
    .unwrap();
    assert_eq!(plan.phases.len(), 1);
    assert_eq!(plan.conflict, Some(0));
    assert_eq!(plan.phases[0].dur, 3.6, "the crash phase lingers");
}

#[test]
fn ns_cars_are_hidden_unless_ns_through_is_green() {
    // This is the bug the user saw: cars must NEVER sit in the road on red/yellow/walk.
    let plan = traffic_plan(&safe_trace()).unwrap();
    assert_eq!(
        ns_drive(&plan, 0),
        NsDrive::Through,
        "ns green → drive through"
    );
    assert_eq!(ns_drive(&plan, 1), NsDrive::Hidden, "ns yellow → hidden");
    assert_eq!(ns_drive(&plan, 2), NsDrive::Hidden, "all-red → hidden");
    assert_eq!(
        ns_drive(&plan, 3),
        NsDrive::Hidden,
        "pedestrian walk → hidden"
    );
}

#[test]
fn the_crash_car_stops_at_the_crosswalk_not_drives_through() {
    let plan = traffic_plan(&wf(
        vec![sig("ns", &[1, 1]), sig("ew", &[0, 0]), sig("ped", &[1, 1])],
        2,
    ))
    .unwrap();
    assert_eq!(ns_drive(&plan, 0), NsDrive::CrashStop);
}

#[test]
fn countdown_counts_down_whole_seconds() {
    assert_eq!(countdown_seq(7.0), vec![7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(countdown_seq(2.6), vec![3, 2, 1]);
    assert_eq!(countdown_seq(1.3), vec![2, 1]);
    assert_eq!(countdown_seq(1.0), vec![1]);
}

#[test]
fn every_animation_name_used_in_the_svg_has_a_matching_keyframes_block() {
    // Guards against the dangling-reference class of bug (body using a keyframe that the
    // style section never defined).
    let svg = traffic_svg(&wf(
        vec![
            sig("ns", &[1, 1, 0, 0, 0, 0]),
            sig("ew", &[0, 0, 0, 1, 1, 0]),
            sig("nsl", &[0, 0, 0, 0, 0, 1]),
            sig("ewl", &[0, 0, 0, 0, 0, 0]),
            sig("ped", &[0, 0, 0, 0, 0, 0]),
        ],
        6,
    ))
    .expect("renders an intersection");
    let mut rest = svg.as_str();
    while let Some(pos) = rest.find("animation-name:") {
        rest = &rest[pos + "animation-name:".len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        assert!(
            svg.contains(&format!("@keyframes {name}{{"))
                || svg.contains(&format!("@keyframes {name} {{")),
            "animation-name {name} has no @keyframes definition"
        );
    }
}

#[test]
fn non_intersection_traces_do_not_render() {
    // No ns/ew signals → not a traffic trace.
    assert!(traffic_svg(&wf(vec![sig("a", &[0, 1]), sig("b", &[1, 0])], 2)).is_none());
}

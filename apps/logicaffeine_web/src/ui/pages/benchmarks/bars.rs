//! The one bar chart: grouped, linear, direct-labelled. The LOGOS family is drawn in steps of the theme's accent (the
//! built tiers in the accent and two tints, the in-process engines in three shades); every other language in one
//! neutral, so the chart's story is LOGOS against the field and a label, not a colour, says which bar is whose.

use super::*;

/// What a bar's number is, and how to write it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BarUnit {
    /// Milliseconds of wall-clock time.
    Millis,
    /// Times the speed of C.
    SpeedOfC,
}

impl BarUnit {
    fn write(self, v: f64) -> String {
        match self {
            BarUnit::Millis => format_time(v),
            BarUnit::SpeedOfC => format!("{v:.2}×"),
        }
    }
}

/// A language's role in every chart's colour: one of the LOGOS tiers, or the rest of the field.
pub(super) fn tier_role(id: &str) -> &'static str {
    match id {
        "logos_release" | "logos_aot" => "logos-rust",
        "logos_c" => "logos-c",
        "logos_cbc" => "logos-cbc",
        "logos_jit" | "logos_interp" | "logos_tiered" => "logos-jit",
        "logos_vm" => "logos-vm",
        "logos_tw" => "logos-tw",
        _ => "other",
    }
}

/// The CSS class that colours a language's bar.
pub(super) fn bar_class(id: &str) -> &'static str {
    match tier_role(id) {
        "logos-rust" => "bench-bar-fill bench-bar-logos-rust",
        "logos-c" => "bench-bar-fill bench-bar-logos-c",
        "logos-cbc" => "bench-bar-fill bench-bar-logos-cbc",
        "logos-jit" => "bench-bar-fill bench-bar-logos-jit",
        "logos-vm" => "bench-bar-fill bench-bar-logos-vm",
        "logos-tw" => "bench-bar-fill bench-bar-logos-tw",
        _ => "bench-bar-fill bench-bar-other",
    }
}

/// The CSS class of a language's colour swatch (a legend dot).
pub(super) fn swatch_class(id: &str) -> &'static str {
    match tier_role(id) {
        "logos-rust" => "bench-swatch bench-swatch-logos-rust",
        "logos-c" => "bench-swatch bench-swatch-logos-c",
        "logos-cbc" => "bench-swatch bench-swatch-logos-cbc",
        "logos-jit" => "bench-swatch bench-swatch-logos-jit",
        "logos-vm" => "bench-swatch bench-swatch-logos-vm",
        "logos-tw" => "bench-swatch bench-swatch-logos-tw",
        _ => "bench-swatch bench-swatch-other",
    }
}

#[component]
pub(super) fn BarChart(groups: Vec<ChartGroup>, unit: BarUnit) -> Element {
    let max = groups
        .iter()
        .flat_map(|g| g.bars.iter().filter_map(|b| b.value))
        .fold(0.0_f64, f64::max)
        .max(f64::MIN_POSITIVE);
    rsx! {
        div { class: "bench-chart",
            for group in groups.iter() {
                div {
                    class: if group.tier == "logos" { "bench-bar-group bench-bar-group-logos" } else { "bench-bar-group" },
                    div { class: "bench-tier-label", "{tier_label(&group.tier)}" }
                    for b in group.bars.iter() {
                        {
                            let class = bar_class(&b.id);
                            match (&b.value, &b.missing) {
                                (Some(v), _) => {
                                    let pct = linear_bar_pct(*v, max);
                                    let text = unit.write(*v);
                                    let inside = pct > 18.0;
                                    rsx! {
                                        div { class: "bench-bar-row", title: "{b.label}: {text}",
                                            div { class: "bench-bar-label", "{b.label}" }
                                            div { class: "bench-bar-track",
                                                div { class: "{class}", style: "width: {pct:.2}%;",
                                                    if inside { span { class: "bench-bar-time", "{text}" } }
                                                }
                                            }
                                            if !inside { span { class: "bench-bar-time-outside", "{text}" } }
                                        }
                                    }
                                }
                                (None, Some(missing)) => {
                                    let why = match missing {
                                        Missing::TimedOut(ms) => format!("timed out ({})", format_timeout(*ms)),
                                        Missing::NotRun(reason) => format!("not run: {reason}"),
                                    };
                                    rsx! {
                                        div { class: "bench-bar-row bench-bar-row-missing", title: "{b.label}: {why}",
                                            div { class: "bench-bar-label", "{b.label}" }
                                            div { class: "bench-bar-track",
                                                div { class: "bench-bar-missing" }
                                            }
                                            span { class: "bench-bar-time-outside", "{why}" }
                                        }
                                    }
                                }
                                (None, None) => rsx! {},
                            }
                        }
                    }
                }
            }
        }
    }
}

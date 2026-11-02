//! Floorplans read off a fitted build: a Logic Lock region for each instance named, a square beside the site its clock
//! comes from, as small as what the instance needs at the parent region's density allows, inside the parent region.
//!
//! The build says everything the floorplan needs. Its clock table names each clock's target cell, and the fitter's
//! fast-signal table the site of each net it put on a fast network: a clock's site is the fast signal sharing its
//! target's deepest hierarchy (an EMIF's user clock leaves its IO tile's control, the net beside it). An instance's clock
//! is the one its timing paths run on, both ends inside it. Its need is the fitter's own entity table (ALMs needed,
//! M20Ks), and the parent region's density its capacity over its sites. The region is then the square that holds the
//! need at a fill (half by default, room for the router), every site of it in the parent region, the nearest such square
//! to the clock's site. Quartus takes it as a `PLACE_REGION` with `CORE_ONLY_PLACE_REGION` on and
//! `RESERVE_PLACE_REGION` off, so the rest of the design may share the sites.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::netlist::Graph;
use super::quartus::{headed_table, number, parse_region_usage, RegionUsage};

/// A rectangle of sites, both corners inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    pub fn sites(&self) -> u64 {
        u64::from(self.x1 - self.x0 + 1) * u64::from(self.y1 - self.y0 + 1)
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }

    /// The Chebyshev distance from a site to the nearest site of the rectangle, 0 inside it.
    pub fn distance(&self, (x, y): (u32, u32)) -> u32 {
        let gap = |v: u32, lo: u32, hi: u32| if v < lo { lo - v } else { v.saturating_sub(hi) };
        gap(x, self.x0, self.x1).max(gap(y, self.y0, self.y1))
    }
}

/// The site of a location the fitter names, `TILECTRL_X233_Y36_N2` its (233, 36).
pub fn site(location: &str) -> Option<(u32, u32)> {
    let field = |key: char| -> Option<u32> {
        let at = location.split('_').find(|p| p.starts_with(key) && p.len() > 1 && p[1..].bytes().all(|b| b.is_ascii_digit()))?;
        at[1..].parse().ok()
    };
    Some((field('X')?, field('Y')?))
}

/// Every net of the fit report's `Global & Other Fast Signals Summary` with the site it is driven from.
pub fn fast_signal_sites(place: &str) -> Vec<(String, (u32, u32))> {
    let Some((header, rows)) = headed_table(place, "Global & Other Fast Signals Summary") else { return Vec::new() };
    let (Some(name), Some(location)) = (header.iter().position(|h| *h == "Name"), header.iter().position(|h| *h == "Location")) else {
        return Vec::new();
    };
    rows.iter().filter_map(|r| Some((r.get(name)?.to_string(), site(r.get(location)?)?))).collect()
}

/// Every clock of the timing analyzer's `Clocks` table with its target cells.
pub fn clock_targets(clocks: &str) -> Vec<(String, Vec<String>)> {
    let Some((header, rows)) = headed_table(clocks, "Clocks") else { return Vec::new() };
    let (Some(name), Some(targets)) = (header.iter().position(|h| *h == "Clock Name"), header.iter().position(|h| *h == "Targets")) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|r| {
            let list = r.get(targets)?.trim().trim_start_matches('{').trim_end_matches('}');
            Some((r.get(name)?.to_string(), list.split_whitespace().map(str::to_string).collect()))
        })
        .collect()
}

/// How many leading hierarchy levels two names share.
fn shared_levels(a: &str, b: &str) -> usize {
    a.split('|').zip(b.split('|')).take_while(|(x, y)| x == y).count()
}

/// The site a clock comes from: the fast signal sharing one of its targets' deepest hierarchy, or none when no fast
/// signal shares any level or two share the deepest.
pub fn clock_site(targets: &[String], fast: &[(String, (u32, u32))]) -> Option<(u32, u32)> {
    let depth = |net: &str| targets.iter().map(|t| shared_levels(t, net)).max().unwrap_or(0);
    let best = fast.iter().map(|(n, _)| depth(n)).max().filter(|&d| d > 0)?;
    let mut at = fast.iter().filter(|(n, _)| depth(n) == best);
    let (_, site) = at.next()?;
    at.next().is_none().then_some(*site)
}

/// The clock an instance runs on: the latch clock of the most setup paths in the timing report whose both ends lie
/// inside it, none when no path does or two clocks tie.
pub fn instance_clock(setup: &str, instance: &str) -> Option<String> {
    let inside = |node: &str| node.strip_prefix(instance).is_some_and(|rest| rest.starts_with('|'));
    let mut counts: Vec<(String, usize)> = Vec::new();
    for line in setup.lines() {
        let Some(c) = cells_of(line) else { continue };
        let [slack, from, to, _, latch, ..] = c.as_slice() else { continue };
        if slack.parse::<f64>().is_err() || !inside(from) || !inside(to) {
            continue;
        }
        match counts.iter_mut().find(|(clock, _)| clock == latch) {
            Some((_, n)) => *n += 1,
            None => counts.push((latch.to_string(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1));
    match counts.as_slice() {
        [(clock, _)] => Some(clock.clone()),
        [(clock, n), (_, m), ..] if n > m => Some(clock.clone()),
        _ => None,
    }
}

fn cells_of(line: &str) -> Option<Vec<&str>> {
    let t = line.trim_end();
    (t.starts_with(';') && t.ends_with(';') && t.len() > 1).then(|| t[1..t.len() - 1].split(';').map(str::trim).collect())
}

/// What an instance needs of a region: its ALMs needed and its M20Ks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Need {
    pub alms: f64,
    pub m20k: u64,
}

/// An instance's need as the fit report's `Fitter Resource Utilization by Entity` states it (its totals, itself and
/// everything under it).
pub fn instance_need(place: &str, instance: &str) -> Option<Need> {
    let (header, rows) = headed_table(place, "Fitter Resource Utilization by Entity")?;
    let column = |name: &str| header.iter().position(|h| *h == name);
    let (alms, m20k, path) = (column("ALMs needed [=A-B+C]")?, column("M20Ks")?, column("Full Hierarchy Name")?);
    let row = rows.iter().find(|r| r.get(path) == Some(&instance))?;
    let total = |cell: &str| cell.split('(').next().map(str::trim).map(str::to_string);
    Some(Need { alms: total(row.get(alms)?)?.parse().ok()?, m20k: number(&total(row.get(m20k)?)?)? })
}

/// A region's resources a site: its ALMs and its M20Ks over its sites.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Density {
    pub alms: f64,
    pub m20k: f64,
}

impl Density {
    /// The density of a region the fitter reported on: what it holds over its rectangles' sites.
    pub fn of(region: &RegionUsage) -> Option<Density> {
        let sites: u64 = region.placement.iter().map(|&((x0, y0), (x1, y1))| Rect { x0, y0, x1, y1 }.sites()).sum();
        let m20k = region.m20k?.total;
        (sites > 0).then(|| Density { alms: region.alms as f64 / sites as f64, m20k: m20k as f64 / sites as f64 })
    }
}

/// The parent region a build's floorplan lies in: the rectangles of its one Logic Lock region (a partial
/// reconfiguration build's persona), with that region's density.
pub fn parent_region(place: &str) -> Option<(Vec<Rect>, Density)> {
    let regions = parse_region_usage(place);
    let [region] = regions.as_slice() else { return None };
    let rects = region.placement.iter().map(|&((x0, y0), (x1, y1))| Rect { x0, y0, x1, y1 }).collect();
    Some((rects, Density::of(region)?))
}

/// The device's sites: the route region of the fit report's `Logic Lock Region Constraints`, the whole core a flat build
/// may place in. It is a row's last cell: the place region before it separates its rectangles with the table's own `;`.
pub fn device(place: &str) -> Option<Rect> {
    let (header, rows) = headed_table(place, "Logic Lock Region Constraints")?;
    if header.last() != Some(&"Route Region Constraint") {
        return None;
    }
    let numbers: Vec<u32> =
        rows.first()?.last()?.split(|c: char| !c.is_ascii_digit()).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect();
    match numbers.as_slice() {
        [x0, y0, x1, y1] => Some(Rect { x0: *x0, y0: *y0, x1: *x1, y1: *y1 }),
        _ => None,
    }
}

/// The square that holds `need` at `fill` of `density`, every site of it in `parent`, nearest `anchor`: the least side
/// whose sites hold the need, then of its placements the one nearest the anchor (Chebyshev), then the one whose centre
/// is nearest, then the lowest. None when no placement of that side fits, or the density holds none of what is needed.
pub fn region(anchor: (u32, u32), need: Need, density: Density, fill: f64, parent: &[Rect]) -> Option<Rect> {
    if parent.is_empty() || !(fill > 0.0 && fill <= 1.0) || density.alms <= 0.0 || (need.m20k > 0 && density.m20k <= 0.0) {
        return None;
    }
    let sites = (need.alms / (density.alms * fill)).max(need.m20k as f64 / (density.m20k * fill).max(f64::MIN_POSITIVE));
    let side = (sites.sqrt().ceil() as u32).max(1);
    let (bx0, by0) = (parent.iter().map(|r| r.x0).min()?, parent.iter().map(|r| r.y0).min()?);
    let (bx1, by1) = (parent.iter().map(|r| r.x1).max()?, parent.iter().map(|r| r.y1).max()?);
    let (w, h) = ((bx1 - bx0 + 1) as usize, (by1 - by0 + 1) as usize);
    if (side as usize) > w || (side as usize) > h {
        return None;
    }
    // Sites of the parent, summed: a square is inside it when its sum is its area.
    let mut sum = vec![0u32; (w + 1) * (h + 1)];
    for y in 0..h {
        for x in 0..w {
            let here = u32::from(parent.iter().any(|r| r.contains(bx0 + x as u32, by0 + y as u32)));
            sum[(y + 1) * (w + 1) + x + 1] = here + sum[y * (w + 1) + x + 1] + sum[(y + 1) * (w + 1) + x] - sum[y * (w + 1) + x];
        }
    }
    let s = side as usize;
    let inside = |x: usize, y: usize| -> bool {
        let at = |xx: usize, yy: usize| sum[yy * (w + 1) + xx];
        at(x + s, y + s) + at(x, y) - at(x, y + s) - at(x + s, y) == side * side
    };
    let centre_gap = |r: &Rect| -> u64 {
        let (cx, cy) = (u64::from(r.x0 + r.x1), u64::from(r.y0 + r.y1));
        cx.abs_diff(2 * u64::from(anchor.0)) + cy.abs_diff(2 * u64::from(anchor.1))
    };
    let mut best: Option<(u32, u64, Rect)> = None;
    for y in 0..=(h - s) {
        for x in 0..=(w - s) {
            if !inside(x, y) {
                continue;
            }
            let r = Rect { x0: bx0 + x as u32, y0: by0 + y as u32, x1: bx0 + (x + s) as u32 - 1, y1: by0 + (y + s) as u32 - 1 };
            let key = (r.distance(anchor), centre_gap(&r));
            if best.as_ref().map_or(true, |(d, c, _)| key < (*d, *c)) {
                best = Some((key.0, key.1, r));
            }
        }
    }
    best.map(|(_, _, r)| r)
}

/// One region: the instances it holds, the name Quartus gives it, its rectangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub instances: Vec<String>,
    pub name: String,
    pub rect: Rect,
}

/// The assignments as Quartus settings: each region a `PLACE_REGION` of every instance it holds, under its one name,
/// core sites only, shared with the rest of the design.
pub fn tcl(assignments: &[Assignment]) -> String {
    let mut out = String::new();
    for a in assignments {
        for instance in &a.instances {
            settings(&mut out, instance, &a.name, &[a.rect]);
        }
    }
    out
}

/// One region's settings: a `PLACE_REGION` of its rectangles (`;` between them, as Quartus writes a region of several),
/// its name, core sites only, shared with the rest of the design.
fn settings(out: &mut String, instance: &str, name: &str, rects: &[Rect]) {
    let place: Vec<String> = rects.iter().map(|r| format!("X{} Y{} X{} Y{}", r.x0, r.y0, r.x1, r.y1)).collect();
    let _ = writeln!(out, "set_instance_assignment -name PLACE_REGION \"{}\" -to {instance}", place.join(";"));
    let _ = writeln!(out, "set_instance_assignment -name REGION_NAME {name} -to {instance}");
    let _ = writeln!(out, "set_instance_assignment -name CORE_ONLY_PLACE_REGION ON -to {instance}");
    let _ = writeln!(out, "set_instance_assignment -name RESERVE_PLACE_REGION OFF -to {instance}");
}

/// What a tile holds, as the fitter counted it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    pub alms: u64,
    pub registers: u64,
    pub m20k: u64,
    pub dsp: u64,
}

/// The device cut into tiles of `width` columns by `height` rows from its corner, row by row, the last of each row and
/// column clipped to the device.
pub fn tiles(device: Rect, width: u32, height: u32) -> Vec<Rect> {
    let mut out = Vec::new();
    for y0 in (device.y0..=device.y1).step_by(height.max(1) as usize) {
        for x0 in (device.x0..=device.x1).step_by(width.max(1) as usize) {
            out.push(Rect { x0, y0, x1: (x0 + width - 1).min(device.x1), y1: (y0 + height - 1).min(device.y1) });
        }
    }
    out
}

/// The geometry probe for `tiles`: a design of one register cell a tile (`c<k>`), and the Logic Lock region pinning each
/// cell to its tile (`geo_<k>`), so a fit of it reports every tile's capacity in its region usage.
pub fn probe(tiles: &[Rect]) -> (String, String) {
    let n = tiles.len();
    let mut v = String::from("module logos_geometry_cell(input clk, input d, output reg q);\n  always @(posedge clk) q <= d;\nendmodule\n\n");
    writeln!(v, "module logos_geometry_probe(input clk, input [{}:0] d, output [{}:0] q);", n.max(1) - 1, n.max(1) - 1).unwrap();
    for k in 0..n {
        writeln!(v, "  logos_geometry_cell c{k}(.clk(clk), .d(d[{k}]), .q(q[{k}]));").unwrap();
    }
    v.push_str("endmodule\n");
    let plan: Vec<Assignment> = tiles.iter().enumerate().map(|(k, &rect)| Assignment { instances: vec![format!("c{k}")], name: format!("geo_{k}"), rect }).collect();
    (v, tcl(&plan))
}

/// The tiles a probe's fit found no flip-flop in: each region its log refuses as needing a flip-flop where there are
/// none (`Deterministic overuse of FF resources in region (x0,y0) to (x1,y1)`), a tile of the device with no logic
/// (hard IP, the core's edge), which the next probe leaves out and no floorplan places a region on.
pub fn fabricless(log: &str) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for line in log.lines() {
        let Some(at) = line.find("Deterministic overuse of FF resources in region ") else { continue };
        let numbers: Vec<u32> = line[at..].split(|c: char| !c.is_ascii_digit()).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect();
        if let [x0, y0, x1, y1] = numbers[..] {
            let r = Rect { x0, y0, x1, y1 };
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out
}

/// The device's capacity map, as a probe's fit counted it: each tile and what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub tiles: Vec<(Rect, Capacity)>,
}

impl Geometry {
    /// The map a probe's fit records: every `geo_<k>` region, one rectangle each, with its counts; none when a region
    /// lacks its counts or no region is a probe's.
    pub fn of(regions: &[RegionUsage]) -> Option<Geometry> {
        let mut tiles = Vec::new();
        for g in regions.iter().filter(|g| g.name.strip_prefix("geo_").is_some_and(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()))) {
            let [((x0, y0), (x1, y1))] = g.placement[..] else { return None };
            let capacity = Capacity { alms: g.alms, registers: g.registers?.total, m20k: g.m20k?.total, dsp: g.dsp?.total };
            tiles.push((Rect { x0, y0, x1, y1 }, capacity));
        }
        (!tiles.is_empty()).then_some(Geometry { tiles })
    }

    /// What `rect` holds when it is a union of whole tiles, each tile's counts summed; none when it cuts a tile or reaches
    /// past the map, a part of a tile being unmeasured.
    pub fn capacity(&self, rect: Rect) -> Option<Capacity> {
        let inside = |t: &Rect| t.x0 >= rect.x0 && t.x1 <= rect.x1 && t.y0 >= rect.y0 && t.y1 <= rect.y1;
        let overlaps = |t: &Rect| t.x0 <= rect.x1 && rect.x0 <= t.x1 && t.y0 <= rect.y1 && rect.y0 <= t.y1;
        if self.tiles.iter().any(|(t, _)| overlaps(t) && !inside(t)) {
            return None;
        }
        let whole: Vec<&(Rect, Capacity)> = self.tiles.iter().filter(|(t, _)| inside(t)).collect();
        if whole.iter().map(|(t, _)| t.sites()).sum::<u64>() != rect.sites() {
            return None;
        }
        Some(whole.iter().fold(Capacity::default(), |c, (_, t)| Capacity {
            alms: c.alms + t.alms,
            registers: c.registers + t.registers,
            m20k: c.m20k + t.m20k,
            dsp: c.dsp + t.dsp,
        }))
    }
}

/// What a unit needs placed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Needs {
    pub alms: f64,
    pub registers: f64,
    pub m20k: u64,
    pub dsp: u64,
}

/// A unit the floorplan places: the instances its one region holds, the region's name, what they need together.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub instances: Vec<String>,
    pub name: String,
    pub needs: Needs,
}

impl Unit {
    /// The name a unit's refusal gives it: its first instance, or its region's name when it holds none.
    fn named(&self) -> String {
        self.instances.first().cloned().unwrap_or_else(|| self.name.clone())
    }
}

/// A bus the floorplan keeps short, weighted by its width in bits: between two units, by their index, or between a unit
/// and a fixed site.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wire {
    Units(usize, usize, u64),
    Site(usize, (u32, u32), u64),
}

/// Why a floorplan places no region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceError {
    /// No rectangle the map measures holds this unit clear of the rest.
    NoRegion(String),
    /// The module has no instance of this unit.
    Unknown(String),
    /// One region name would serve both units.
    SameName(String, String),
    /// A unit of no instance, by its name.
    Empty(String),
    /// An instance two units hold.
    Twice(String),
}

impl std::fmt::Display for PlaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlaceError::NoRegion(unit) => write!(f, "{unit}: no free whole tiles hold it clear of the regions before it"),
            PlaceError::Unknown(unit) => write!(f, "{unit}: the module has no such instance"),
            PlaceError::SameName(a, b) => write!(f, "{a} and {b}: one region name for both"),
            PlaceError::Empty(name) => write!(f, "{name}: a unit of no instance"),
            PlaceError::Twice(instance) => write!(f, "{instance}: in two units"),
        }
    }
}

/// How much of a region a unit may take of each resource: of its logic (ALMs and registers), the rest room for the router
/// and for the logic that shares the region; of its M20Ks and its DSP blocks, hard blocks that need no such room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub logic: f64,
    pub m20k: f64,
    pub dsp: f64,
}

impl Fill {
    /// One share of every resource.
    pub fn uniform(share: f64) -> Fill {
        Fill { logic: share, m20k: share, dsp: share }
    }
}

/// Whether a region of `capacity` holds a unit of `needs` at `fill`: each resource the unit needs within its share of the
/// region's.
pub fn holds(capacity: Capacity, needs: Needs, fill: Fill) -> bool {
    needs.alms <= fill.logic * capacity.alms as f64
        && needs.registers <= fill.logic * capacity.registers as f64
        && needs.m20k as f64 <= fill.m20k * capacity.m20k as f64
        && needs.dsp as f64 <= fill.dsp * capacity.dsp as f64
}

/// The map's tiles as a grid: the tiles' distinct columns and rows, each cell's capacity (none where no tile is), and
/// the prefix sums of every count, so a block of whole tiles is summed at once.
struct Grid {
    xs: Vec<(u32, u32)>,
    ys: Vec<(u32, u32)>,
    sums: Vec<[u64; 5]>,
}

impl Grid {
    fn of(geometry: &Geometry) -> Grid {
        let mut xs: Vec<(u32, u32)> = geometry.tiles.iter().map(|(t, _)| (t.x0, t.x1)).collect();
        let mut ys: Vec<(u32, u32)> = geometry.tiles.iter().map(|(t, _)| (t.y0, t.y1)).collect();
        xs.sort_unstable();
        xs.dedup();
        ys.sort_unstable();
        ys.dedup();
        let (w, h) = (xs.len(), ys.len());
        let mut sums = vec![[0u64; 5]; (w + 1) * (h + 1)];
        let at = |i: usize, j: usize| j * (w + 1) + i;
        for (t, c) in &geometry.tiles {
            let (Ok(i), Ok(j)) = (xs.binary_search(&(t.x0, t.x1)), ys.binary_search(&(t.y0, t.y1))) else { continue };
            sums[at(i + 1, j + 1)] = [c.alms, c.registers, c.m20k, c.dsp, 1];
        }
        for j in 1..=h {
            for i in 1..=w {
                for k in 0..5 {
                    sums[at(i, j)][k] = sums[at(i, j)][k] + sums[at(i - 1, j)][k] + sums[at(i, j - 1)][k] - sums[at(i - 1, j - 1)][k];
                }
            }
        }
        Grid { xs, ys, sums }
    }

    /// The tiles from column `i0` to `i1` and row `j0` to `j1`: their rectangle and capacity, none if a cell has no
    /// tile.
    fn block(&self, i0: usize, i1: usize, j0: usize, j1: usize) -> Option<(Rect, Capacity)> {
        let w = self.xs.len() + 1;
        let s = |i: usize, j: usize| self.sums[j * w + i];
        let k = |n: usize| s(i1 + 1, j1 + 1)[n] + s(i0, j0)[n] - s(i0, j1 + 1)[n] - s(i1 + 1, j0)[n];
        if k(4) != ((i1 - i0 + 1) * (j1 - j0 + 1)) as u64 {
            return None;
        }
        let rect = Rect { x0: self.xs[i0].0, y0: self.ys[j0].0, x1: self.xs[i1].1, y1: self.ys[j1].1 };
        Some((rect, Capacity { alms: k(0), registers: k(1), m20k: k(2), dsp: k(3) }))
    }
}

fn overlap(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Every rectangle of whole tiles of `geometry` that holds `needs` at `fill`, overlaps none of `blocked`, is no longer
/// than `aspect` times its width in sites, and is least: no rectangle a tile row or column smaller inside it holds the
/// needs too. In order of their top row, then left column, then size.
pub fn candidates(geometry: &Geometry, needs: Needs, fill: Fill, aspect: f64, blocked: &[Rect]) -> Vec<Rect> {
    let grid = Grid::of(geometry);
    let (w, h) = (grid.xs.len(), grid.ys.len());
    let holding = |i0: usize, i1: usize, j0: usize, j1: usize| grid.block(i0, i1, j0, j1).is_some_and(|(_, c)| holds(c, needs, fill));
    let mut out = Vec::new();
    for j0 in 0..h {
        for i0 in 0..w {
            for j1 in j0..h {
                for i1 in i0..w {
                    let Some((rect, capacity)) = grid.block(i0, i1, j0, j1) else { continue };
                    if !holds(capacity, needs, fill) || blocked.iter().any(|b| overlap(&rect, b)) {
                        continue;
                    }
                    let (long, short) = {
                        let (x, y) = ((rect.x1 - rect.x0 + 1) as f64, (rect.y1 - rect.y0 + 1) as f64);
                        (x.max(y), x.min(y))
                    };
                    if long > aspect * short {
                        continue;
                    }
                    let smaller = (i1 > i0 && (holding(i0 + 1, i1, j0, j1) || holding(i0, i1 - 1, j0, j1)))
                        || (j1 > j0 && (holding(i0, i1, j0 + 1, j1) || holding(i0, i1, j0, j1 - 1)));
                    if !smaller {
                        out.push(rect);
                    }
                }
            }
        }
    }
    out
}

/// The centre of `r`, in sites.
fn centre(r: &Rect) -> (f64, f64) {
    (f64::from(r.x0 + r.x1) / 2.0, f64::from(r.y0 + r.y1) / 2.0)
}

/// The floorplan of `units`, each in the order given at its candidate of least cost: the width of each of its wires to
/// a unit placed before it or to a fixed site, times the straight distance between centres, so a unit sits where its
/// wires meet; a candidate overlaps no unit placed before it nor `blocked`. Ties go to the smaller perimeter (the more
/// compact region), then area, then the top row and left column. A unit no candidate holds is refused by name.
pub fn place(geometry: &Geometry, units: &[Unit], wires: &[Wire], fill: Fill, aspect: f64, blocked: &[Rect]) -> Result<Vec<Assignment>, PlaceError> {
    let mut plan: Vec<Assignment> = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        let taken: Vec<Rect> = blocked.iter().copied().chain(plan.iter().map(|a| a.rect)).collect();
        let cost = |r: &Rect| -> f64 {
            let (cx, cy) = centre(r);
            let to = |(x, y): (f64, f64)| ((cx - x).powi(2) + (cy - y).powi(2)).sqrt();
            wires
                .iter()
                .map(|w| match *w {
                    Wire::Units(a, b, width) if (a == k && b < k) || (b == k && a < k) => {
                        let other = if a == k { b } else { a };
                        width as f64 * to(centre(&plan[other].rect))
                    }
                    Wire::Site(u, (x, y), width) if u == k => width as f64 * to((f64::from(x), f64::from(y))),
                    _ => 0.0,
                })
                .sum()
        };
        let best = candidates(geometry, unit.needs, fill, aspect, &taken)
            .into_iter()
            .map(|r| (cost(&r), (r.x1 - r.x0) + (r.y1 - r.y0), r.sites(), r))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then((a.3.y0, a.3.x0).cmp(&(b.3.y0, b.3.x0))))
            .ok_or_else(|| PlaceError::NoRegion(unit.named()))?;
        plan.push(Assignment { instances: unit.instances.clone(), name: unit.name.clone(), rect: best.3 });
    }
    Ok(plan)
}

/// The M20K blocks a memory of `depth` words of `width` bits takes: the fewest over the block's simple-dual-port shapes
/// (512 × 40, 1,024 × 20, 2,048 × 10), as many blocks side by side as its width needs and stacked as its depth does, the
/// stack a power of two deep (the fitter decodes it by the address's top bits).
pub fn m20k_blocks(depth: u64, width: u64) -> u64 {
    blocks(depth, width, None)
}

/// The M20K blocks of `m20k_blocks`, with byte enables of `byte` bits a block's every 10-bit lane holding one byte.
fn blocks(depth: u64, width: u64, byte: Option<u64>) -> u64 {
    let stack = depth.max(1).next_power_of_two();
    [(512u64, 40u64), (1024, 20), (2048, 10)]
        .iter()
        .map(|&(d, w)| width.max(1).div_ceil(byte.map_or(w, |b| w / 10 * b.clamp(1, 10))) * stack.div_ceil(d))
        .min()
        .expect("three shapes")
}

/// A memory as synthesis left it: its path, the block it asked for (`AUTO`, `M20K block`, `MLAB`), each port's depth and
/// width, its byte enables' byte when it has them, and whether a read on one port during a write on the other must give
/// the old data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ram {
    pub name: String,
    pub kind: String,
    pub ports: Vec<(u64, u64)>,
    pub byte: Option<u64>,
    pub old_data: bool,
}

impl Ram {
    /// The M20K blocks the fitter gives the memory: none for one asked into MLABs, none for an `AUTO` one the fitter
    /// turns into MLABs, and otherwise its costlier port's blocks (`m20k_blocks`, byte-enabled lanes holding a byte
    /// each). The fitter turns an `AUTO` memory into MLABs when it is small (every port 32 words of at most 42 bits, 64
    /// of at most 10, or 128 of at most 5, as hybrid7b's fit did) and needs no old data across its ports, which an MLAB
    /// cannot give.
    pub fn m20k(&self) -> u64 {
        let small = |&(d, w): &(u64, u64)| (d <= 32 && w <= 42) || (d <= 64 && w <= 10) || (d <= 128 && w <= 5);
        let mlab = self.kind == "MLAB" || (self.kind == "AUTO" && !self.old_data && self.ports.iter().all(small));
        if mlab {
            return 0;
        }
        self.ports.iter().map(|&(d, w)| blocks(d, w, self.byte)).max().unwrap_or(0)
    }
}

/// Every memory of the synthesis report's RAM summaries (one a partition), each with what its inferred parameter section
/// (`Parameter Settings for Inferred Entity Instance: <path>`, the path its name continues) says of its byte enables and
/// of a read across its ports; a memory no section names has neither.
pub fn synthesized_rams(syn: &str) -> Vec<Ram> {
    let mut sections: std::collections::HashMap<&str, Vec<(&str, &str)>> = std::collections::HashMap::new();
    let mut lines = syn.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(path) = cells_of(line).filter(|c| c.len() == 1).and_then(|c| c[0].strip_prefix("Parameter Settings for Inferred Entity Instance: ")) else {
            continue;
        };
        let mut rows = Vec::new();
        while let Some(next) = lines.peek() {
            if next.starts_with('+') {
                lines.next();
                continue;
            }
            let Some(c) = cells_of(next) else { break };
            if let [name, value, ..] = c[..] {
                rows.push((name, value));
            }
            lines.next();
        }
        sections.insert(path, rows);
    }
    let mut titles: Vec<&str> = syn
        .lines()
        .filter_map(cells_of)
        .filter(|c| c.len() == 1 && c[0].starts_with("Synthesis RAM Summary for Partition "))
        .map(|c| c[0])
        .collect();
    titles.dedup();
    let mut rams = Vec::new();
    for title in titles {
        let Some((header, rows)) = headed_table(syn, title) else { continue };
        let col = |name: &str| header.iter().position(|h| *h == name);
        let (Some(name), Some(kind), Some(da), Some(wa), Some(db), Some(wb)) =
            (col("Name"), col("Type"), col("Port A Depth"), col("Port A Width"), col("Port B Depth"), col("Port B Width"))
        else {
            continue;
        };
        rams.extend(rows.iter().filter_map(|r| {
            let path = *r.get(name)?;
            let port = |d: usize, w: usize| Some((number(r.get(d)?)?, number(r.get(w)?)?));
            let ports: Vec<(u64, u64)> = port(da, wa).into_iter().chain(port(db, wb)).collect();
            let params = path.match_indices('|').rev().find_map(|(at, _)| sections.get(&path[..at]));
            let value = |key: &str| params.and_then(|p| p.iter().find(|(k, _)| *k == key)).map(|(_, v)| *v);
            let enabled = ["WIDTH_BYTEENA_A", "WIDTH_BYTEENA_B"].iter().any(|k| value(k).and_then(number).is_some_and(|n| n > 1));
            let byte = enabled.then(|| value("BYTE_SIZE").and_then(number).unwrap_or(8));
            let old_data = value("READ_DURING_WRITE_MODE_MIXED_PORTS") == Some("OLD_DATA");
            Some(Ram { name: path.to_string(), kind: r.get(kind)?.to_string(), ports, byte, old_data })
        }));
    }
    rams
}

/// What the instances `members` need placed together as Quartus synthesized them, from the synthesis report `syn`: the
/// entity table's ALUTs, registers and DSP blocks of each member's whole subtree summed, those ALUTs and registers in
/// ALMs at `packing` packed as one (the fitter packs one member's registers beside another's LUTs), and the M20K blocks
/// the fitter gives every memory under any of them (`Ram::m20k`). The member the entity table does not name, when one
/// is missing.
pub fn synthesized_needs<S: AsRef<str>>(syn: &str, members: &[S], packing: &super::preflight::Packing) -> Result<Needs, String> {
    let first = |cell: &str| number(cell.split('(').next()?);
    let table = headed_table(syn, "Partition \"root_partition\" Resource Utilization by Entity");
    let counts = |instance: &str| {
        let (header, rows) = table.as_ref()?;
        let col = |name: &str| header.iter().position(|h| *h == name);
        let (aluts, registers, dsp, path) = (col("Combinational ALUTs")?, col("Dedicated Logic Registers")?, col("DSP Blocks")?, col("Full Hierarchy Name")?);
        let row = rows.iter().find(|r| r.get(path) == Some(&instance))?;
        Some((first(row.get(aluts)?)?, first(row.get(registers)?)?, first(row.get(dsp)?)?))
    };
    let rams = synthesized_rams(syn);
    let (mut aluts, mut registers, mut dsp, mut m20k) = (0, 0, 0, 0);
    for member in members.iter().map(AsRef::as_ref) {
        let (a, r, d) = counts(member).ok_or_else(|| member.to_string())?;
        let under = format!("{member}|");
        (aluts, registers, dsp) = (aluts + a, registers + r, dsp + d);
        m20k += rams.iter().filter(|ram| ram.name.starts_with(&under)).map(Ram::m20k).sum::<u64>();
    }
    Ok(Needs { alms: packing.alms(aluts as f64, registers as f64), registers: registers as f64, m20k, dsp })
}

/// Where an instance's clock comes from: the instance, the clock its paths run on, and the site that clock leaves (a
/// memory channel's, its bank's IO tile control).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub instance: String,
    pub clock: String,
    pub site: (u32, u32),
}

/// Why a build's reports place no anchor for an instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorError {
    /// No one clock the instance's setup paths run on.
    NoClock(String),
    /// The instance's clock is not in the clock table.
    NoTargets(String),
    /// No one fast signal the clock comes from.
    NoSite(String),
}

impl std::fmt::Display for AnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnchorError::NoClock(instance) => write!(f, "{instance}: no one clock its setup paths run on"),
            AnchorError::NoTargets(clock) => write!(f, "{clock}: not in the clock table"),
            AnchorError::NoSite(clock) => write!(f, "{clock}: no one fast signal it comes from"),
        }
    }
}

/// The anchors ledger (`benchmarks/silicon/d5005/emif_anchors.json`): what it holds, the build whose reports placed the
/// anchors, the day they were recorded, and the anchors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchors {
    pub what: String,
    pub build: String,
    pub recorded: String,
    pub anchors: Vec<Anchor>,
}

/// The anchor of `instance` from a fitted build's reports: the clock its setup paths run on (`instance_clock`), that
/// clock's targets in the clock table, and the fast signal of the place report those targets come from (`clock_site`).
pub fn anchor(place: &str, clocks: &str, setup: &str, instance: &str) -> Result<Anchor, AnchorError> {
    let clock = instance_clock(setup, instance).ok_or_else(|| AnchorError::NoClock(instance.to_string()))?;
    let table = clock_targets(clocks);
    let (_, targets) = table.iter().find(|(c, _)| *c == clock).ok_or_else(|| AnchorError::NoTargets(clock.clone()))?;
    let site = clock_site(targets, &fast_signal_sites(place)).ok_or_else(|| AnchorError::NoSite(clock.clone()))?;
    Ok(Anchor { instance: instance.to_string(), clock, site })
}

/// A Logic Lock region's name for an instance path: `logos_` and the path, its levels and a generate block's scope parted
/// by `_`, an index's brackets dropped (`ch[0].channel` is `logos_ch0_channel`).
pub fn region_name(path: &str) -> String {
    let mut name = String::from("logos_");
    for c in path.chars() {
        match c {
            '[' | ']' => {}
            c if c.is_ascii_alphanumeric() || c == '_' => name.push(c),
            _ => name.push('_'),
        }
    }
    name
}

/// The units a floorplan places and its wires, from a module's graph: each of `units` (its instances paths under the
/// module `graph` was read from, its region's name, what its instances need together) placed under `under` (the
/// module's own path in the build); a wire between two units of the bits the graph counts between an instance of the
/// one and an instance of the other (bits between two instances of one unit stay inside its region); and from a unit to
/// an anchor (matched by an instance's full path) of the bits that instance shares with the module's ports, the way out
/// to the anchor. Refused by name: an instance the module lacks, a unit of none, an instance two units hold, and two
/// units of one region name.
pub fn unit_wires(graph: &Graph, under: &str, units: &[Unit], anchors: &[Anchor]) -> Result<(Vec<Unit>, Vec<Wire>), PlaceError> {
    let full = |path: &str| if under.is_empty() { path.to_string() } else { format!("{under}|{path}") };
    let mut placing: Vec<Unit> = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        if unit.instances.is_empty() {
            return Err(PlaceError::Empty(unit.name.clone()));
        }
        if let Some(path) = unit.instances.iter().find(|p| !graph.instances.contains(p)) {
            return Err(PlaceError::Unknown(path.clone()));
        }
        if let Some(path) = unit.instances.iter().enumerate().find(|(i, p)| units[..k].iter().any(|u| u.instances.contains(p)) || unit.instances[..*i].contains(p)).map(|(_, p)| p) {
            return Err(PlaceError::Twice(path.clone()));
        }
        if let Some(other) = units[..k].iter().find(|u| u.name == unit.name) {
            return Err(PlaceError::SameName(other.named(), unit.named()));
        }
        placing.push(Unit { instances: unit.instances.iter().map(|p| full(p)).collect(), name: unit.name.clone(), needs: unit.needs });
    }
    let index = |n: &str| units.iter().position(|u| u.instances.iter().any(|p| p == n));
    let mut wires: Vec<Wire> = graph
        .links
        .iter()
        .filter_map(|l| Some((index(&l.a)?, index(&l.b)?, l.bits)))
        .filter(|(a, b, _)| a != b)
        .map(|(a, b, bits)| Wire::Units(a, b, bits))
        .collect();
    for (k, unit) in units.iter().enumerate() {
        for path in &unit.instances {
            let out = graph.ports.iter().find(|(n, _)| n == path).map_or(0, |(_, bits)| *bits);
            wires.extend(anchors.iter().filter(|a| a.instance == full(path)).map(|a| Wire::Site(k, a.site, out)));
        }
    }
    Ok((placing, wires))
}

/// The instances of `graph` no unit of `units` holds, each with the bits its links carry to the units' instances,
/// heaviest first (then by path), those carrying none left out: a path between two units through an instance no unit
/// holds is no wire of the floorplan's (`unit_wires` counts links between units' instances alone) until its module is
/// dissolved into the graph (`netlist::module_graph`'s `through`).
pub fn loose(graph: &Graph, units: &[Unit]) -> Vec<(String, u64)> {
    let held = |n: &str| units.iter().any(|u| u.instances.iter().any(|p| p == n));
    let mut carried: Vec<(String, u64)> = graph
        .instances
        .iter()
        .filter(|i| !held(i))
        .map(|i| {
            let bits = graph.links.iter().filter(|l| (l.a == *i && held(&l.b)) || (l.b == *i && held(&l.a))).map(|l| l.bits).sum();
            (i.clone(), bits)
        })
        .filter(|(_, bits)| *bits > 0)
        .collect();
    carried.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
    carried
}

/// The floorplan of `units` in rectangles (`place`) on the wires `unit_wires` reads off the module's graph.
#[allow(clippy::too_many_arguments)]
pub fn unit_floorplan(
    geometry: &Geometry,
    graph: &Graph,
    under: &str,
    units: &[Unit],
    anchors: &[Anchor],
    fill: Fill,
    aspect: f64,
    blocked: &[Rect],
) -> Result<Vec<Assignment>, PlaceError> {
    let (placing, wires) = unit_wires(graph, under, units, anchors)?;
    place(geometry, &placing, &wires, fill, aspect, blocked)
}

/// The floorplan of `units` in regions of whole tiles (`grow`) on the wires `unit_wires` reads off the module's graph.
pub fn unit_regions(
    geometry: &Geometry,
    graph: &Graph,
    under: &str,
    units: &[Unit],
    anchors: &[Anchor],
    fill: Fill,
    blocked: &[Rect],
) -> Result<Vec<Region>, PlaceError> {
    let (placing, wires) = unit_wires(graph, under, units, anchors)?;
    grow(geometry, &placing, &wires, fill, blocked)
}

/// A unit's region of whole tiles: the instance, its name, its tiles in the map's order (top row first, then left).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub instances: Vec<String>,
    pub name: String,
    pub tiles: Vec<Rect>,
}

impl Region {
    /// The rectangles the region's tiles merge into: each row's runs of tiles side by side, then runs of one span in rows
    /// one above the other, in the map's order.
    pub fn rects(&self) -> Vec<Rect> {
        let mut tiles = self.tiles.clone();
        tiles.sort_by_key(|t| (t.y0, t.x0));
        let mut runs: Vec<Rect> = Vec::new();
        for t in tiles {
            match runs.last_mut() {
                Some(r) if r.y0 == t.y0 && r.y1 == t.y1 && r.x1 + 1 == t.x0 => r.x1 = t.x1,
                _ => runs.push(t),
            }
        }
        runs.sort_by_key(|r| (r.x0, r.x1, r.y0));
        let mut merged: Vec<Rect> = Vec::new();
        for r in runs {
            match merged.last_mut() {
                Some(m) if m.x0 == r.x0 && m.x1 == r.x1 && m.y1 + 1 == r.y0 => m.y1 = r.y1,
                _ => merged.push(r),
            }
        }
        merged.sort_by_key(|r| (r.y0, r.x0));
        merged
    }

    /// The centre of the region's sites.
    fn centre(&self) -> (f64, f64) {
        let sites: f64 = self.tiles.iter().map(|t| t.sites() as f64).sum();
        let (x, y) = self.tiles.iter().fold((0.0, 0.0), |(x, y), t| {
            let (cx, cy) = centre(t);
            (x + cx * t.sites() as f64, y + cy * t.sites() as f64)
        });
        (x / sites, y / sites)
    }
}

/// The regions as Quartus settings: each a `PLACE_REGION` of its rectangles (`Region::rects`), its own name, core sites
/// only, shared with the rest of the design.
pub fn regions_tcl(regions: &[Region]) -> String {
    let mut out = String::new();
    for r in regions {
        let rects = r.rects();
        for instance in &r.instances {
            settings(&mut out, instance, &r.name, &rects);
        }
    }
    out
}

/// The floorplan of `units` in regions of whole tiles, each unit in the order given. Its seed is the free tile where its
/// wires are shortest (each wire's width times the straight distance from the tile's centre to the centre of the unit
/// at its other end, placed before it, or to its fixed site), among the free tiles of an area that holds it at `fill`,
/// the top row and left column first on a tie. The region then takes, one at a time, the free tile beside it nearest
/// the centre of its sites so far (then where the wires are shortest, then the top row and left column) until it holds
/// the unit, so it grows as compact as the tiles allow and bends round what is taken. A tile is free when no region
/// before it took it and it overlaps nothing `blocked`. A unit no free area holds is refused by name.
pub fn grow(geometry: &Geometry, units: &[Unit], wires: &[Wire], fill: Fill, blocked: &[Rect]) -> Result<Vec<Region>, PlaceError> {
    let tiles: Vec<Rect> = geometry.tiles.iter().map(|(r, _)| *r).collect();
    let n = tiles.len();
    let beside = |a: &Rect, b: &Rect| {
        ((a.x1 + 1 == b.x0 || b.x1 + 1 == a.x0) && a.y0 <= b.y1 && b.y0 <= a.y1)
            || ((a.y1 + 1 == b.y0 || b.y1 + 1 == a.y0) && a.x0 <= b.x1 && b.x0 <= a.x1)
    };
    let neighbours: Vec<Vec<usize>> = (0..n).map(|a| (0..n).filter(|&b| beside(&tiles[a], &tiles[b])).collect()).collect();
    let mut taken: Vec<bool> = tiles.iter().map(|t| blocked.iter().any(|b| overlap(t, b))).collect();
    let add = |c: Capacity, t: Capacity| Capacity { alms: c.alms + t.alms, registers: c.registers + t.registers, m20k: c.m20k + t.m20k, dsp: c.dsp + t.dsp };
    let at = |t: usize| (tiles[t].y0, tiles[t].x0);
    let mut plan: Vec<Region> = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        let cost = |t: usize| -> f64 {
            let (cx, cy) = centre(&tiles[t]);
            let to = |(x, y): (f64, f64)| ((cx - x).powi(2) + (cy - y).powi(2)).sqrt();
            wires
                .iter()
                .map(|w| match *w {
                    Wire::Units(a, b, width) if (a == k && b < k) || (b == k && a < k) => {
                        width as f64 * to(plan[if a == k { b } else { a }].centre())
                    }
                    Wire::Site(u, (x, y), width) if u == k => width as f64 * to((f64::from(x), f64::from(y))),
                    _ => 0.0,
                })
                .sum()
        };
        let costs: Vec<f64> = (0..n).map(cost).collect();
        // The free areas: each free tile's area, and whether the area holds the unit.
        let mut area = vec![usize::MAX; n];
        let mut holding: Vec<bool> = Vec::new();
        for t in 0..n {
            if taken[t] || area[t] != usize::MAX {
                continue;
            }
            let (id, mut stack, mut have) = (holding.len(), vec![t], Capacity::default());
            area[t] = id;
            while let Some(a) = stack.pop() {
                have = add(have, geometry.tiles[a].1);
                for &b in &neighbours[a] {
                    if !taken[b] && area[b] == usize::MAX {
                        area[b] = id;
                        stack.push(b);
                    }
                }
            }
            holding.push(holds(have, unit.needs, fill));
        }
        let seed = (0..n)
            .filter(|&t| !taken[t] && holding[area[t]])
            .min_by(|&a, &b| costs[a].total_cmp(&costs[b]).then(at(a).cmp(&at(b))))
            .ok_or_else(|| PlaceError::NoRegion(unit.named()))?;
        let mut inside = vec![false; n];
        inside[seed] = true;
        let (mut region, mut have) = (vec![seed], geometry.tiles[seed].1);
        let weigh = |t: usize| {
            let ((x, y), w) = (centre(&tiles[t]), tiles[t].sites() as f64);
            (x * w, y * w, w)
        };
        let mut mass = weigh(seed);
        while !holds(have, unit.needs, fill) {
            let at_centre = (mass.0 / mass.2, mass.1 / mass.2);
            let gap = |t: usize| {
                let (x, y) = centre(&tiles[t]);
                ((x - at_centre.0).powi(2) + (y - at_centre.1).powi(2)).sqrt()
            };
            let next = region
                .iter()
                .flat_map(|&t| neighbours[t].iter().copied())
                .filter(|&b| !taken[b] && !inside[b])
                .min_by(|&a, &b| gap(a).total_cmp(&gap(b)).then(costs[a].total_cmp(&costs[b])).then(at(a).cmp(&at(b))))
                .expect("the seed's area holds the unit");
            inside[next] = true;
            region.push(next);
            have = add(have, geometry.tiles[next].1);
            let w = weigh(next);
            mass = (mass.0 + w.0, mass.1 + w.1, mass.2 + w.2);
        }
        region.iter().for_each(|&t| taken[t] = true);
        region.sort_by_key(|&t| at(t));
        plan.push(Region { instances: unit.instances.clone(), name: unit.name.clone(), tiles: region.into_iter().map(|t| tiles[t]).collect() });
    }
    Ok(plan)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/floorplan/tests.rs"]
mod tests;

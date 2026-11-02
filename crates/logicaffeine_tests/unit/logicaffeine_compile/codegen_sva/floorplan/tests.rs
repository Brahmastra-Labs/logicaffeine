use super::*;
use crate::codegen_sva::netlist::Graph;
use crate::codegen_sva::quartus::Ratio;

#[test]
fn a_location_is_its_site_and_a_rectangle_measures_distance_from_its_nearest_site() {
    assert_eq!(site("TILECTRL_X233_Y36_N2"), Some((233, 36)));
    assert_eq!(site("C2_X197_Y382_N0_I28"), Some((197, 382)));
    assert_eq!(site("no site here"), None);
    let r = Rect { x0: 10, y0: 20, x1: 19, y1: 29 };
    assert_eq!(r.sites(), 100);
    assert_eq!((r.distance((15, 25)), r.distance((5, 25)), r.distance((25, 40)), r.distance((19, 29))), (0, 5, 11, 0));
}

#[test]
fn a_clocks_site_is_the_one_fast_signal_sharing_its_targets_deepest_hierarchy() {
    let fast = vec![("a|b|io|net".to_string(), (1, 2)), ("a|c|io|net".to_string(), (3, 4)), ("z|net".to_string(), (5, 6))];
    assert_eq!(clock_site(&["a|b|io|tile|out".into()], &fast), Some((1, 2)));
    assert_eq!(clock_site(&["q|out".into()], &fast), None, "no level shared");
    let tied = vec![("a|b|x".to_string(), (1, 2)), ("a|b|y".to_string(), (3, 4))];
    assert_eq!(clock_site(&["a|b|z".into()], &tied), None, "two at the deepest");
}

#[test]
fn the_region_is_the_least_square_holding_the_need_inside_the_parent_and_nearest_the_anchor() {
    let parent = [Rect { x0: 0, y0: 0, x1: 99, y1: 99 }];
    let density = Density { alms: 10.0, m20k: 0.1 };
    // 40 M20Ks at half fill need 800 sites: a side of 29; 500 ALMs need only 100.
    let r = region((99, 0), Need { alms: 500.0, m20k: 40 }, density, 0.5, &parent).unwrap();
    assert_eq!((r.x1 - r.x0 + 1, r.y1 - r.y0 + 1), (29, 29));
    assert_eq!((r.x1, r.y0), (99, 0), "against the anchor's corner");
    assert_eq!(region((99, 0), Need { alms: 500.0, m20k: 40 }, density, 0.5, &parent), Some(r), "the same square every time");
    // An anchor outside the parent: the nearest square inside it.
    let notched = [Rect { x0: 0, y0: 0, x1: 49, y1: 99 }, Rect { x0: 50, y0: 50, x1: 99, y1: 99 }];
    let n = region((99, 0), Need { alms: 100.0, m20k: 0 }, density, 0.5, &notched).unwrap();
    for x in n.x0..=n.x1 {
        for y in n.y0..=n.y1 {
            assert!(notched.iter().any(|p| p.contains(x, y)), "({x}, {y}) of {n:?} is in the parent");
        }
    }
    // The bottom of the left half and the foot of the upper right are each 50 away with equal centre gaps: the lowest.
    assert_eq!(n, Rect { x0: 45, y0: 0, x1: 49, y1: 4 }, "as near as the notch allows");
    assert_eq!(region((0, 0), Need { alms: 1e9, m20k: 0 }, density, 0.5, &parent), None, "more than the parent holds");
    assert_eq!(region((0, 0), Need { alms: 1.0, m20k: 1 }, Density { alms: 10.0, m20k: 0.0 }, 0.5, &parent), None, "M20Ks where there are none");
}

#[test]
fn the_settings_place_each_instance_in_its_region_on_core_sites_shared_with_the_design() {
    let a = Assignment { instances: vec!["top|ch[0].channel".into()], name: "logos_ch0".into(), rect: Rect { x0: 1, y0: 2, x1: 3, y1: 4 } };
    assert_eq!(
        tcl(&[a]),
        "set_instance_assignment -name PLACE_REGION \"X1 Y2 X3 Y4\" -to top|ch[0].channel\n\
         set_instance_assignment -name REGION_NAME logos_ch0 -to top|ch[0].channel\n\
         set_instance_assignment -name CORE_ONLY_PLACE_REGION ON -to top|ch[0].channel\n\
         set_instance_assignment -name RESERVE_PLACE_REGION OFF -to top|ch[0].channel\n"
    );
}

#[test]
fn the_tiles_cover_the_device_once_the_last_of_each_row_and_column_clipped() {
    let device = Rect { x0: 0, y0: 0, x1: 282, y1: 432 };
    let t = tiles(device, 16, 24);
    assert_eq!(t.len(), 18 * 19, "283 columns in 16s, 433 rows in 24s");
    assert_eq!(t.iter().map(Rect::sites).sum::<u64>(), device.sites(), "every site once");
    for (i, a) in t.iter().enumerate() {
        assert!(a.x1 <= device.x1 && a.y1 <= device.y1 && a.x0 <= a.x1 && a.y0 <= a.y1, "{a:?}");
        for b in &t[i + 1..] {
            assert!(a.x1 < b.x0 || b.x1 < a.x0 || a.y1 < b.y0 || b.y1 < a.y0, "{a:?} and {b:?} apart");
        }
    }
    assert_eq!(t[0], Rect { x0: 0, y0: 0, x1: 15, y1: 23 });
    assert_eq!(*t.last().unwrap(), Rect { x0: 272, y0: 432, x1: 282, y1: 432 }, "clipped to the device");
}

#[test]
fn the_probe_is_a_register_cell_a_tile_each_pinned_to_its_tile() {
    let t = tiles(Rect { x0: 0, y0: 0, x1: 31, y1: 47 }, 16, 24);
    let (verilog, settings) = probe(&t);
    assert_eq!(verilog.matches("logos_geometry_cell c").count(), t.len(), "{verilog}");
    assert!(verilog.contains("module logos_geometry_probe(input clk, input [3:0] d, output [3:0] q);"), "{verilog}");
    for (k, r) in t.iter().enumerate() {
        assert!(verilog.contains(&format!("logos_geometry_cell c{k}(.clk(clk), .d(d[{k}]), .q(q[{k}]));")));
        assert!(settings.contains(&format!("set_instance_assignment -name PLACE_REGION \"X{} Y{} X{} Y{}\" -to c{k}\n", r.x0, r.y0, r.x1, r.y1)));
        assert!(settings.contains(&format!("set_instance_assignment -name REGION_NAME geo_{k} -to c{k}\n")));
    }
}

fn tile_region(k: usize, r: Rect, m20k: u64) -> RegionUsage {
    RegionUsage {
        name: format!("geo_{k}"),
        alms_needed: 1.0,
        alms: 100 * r.sites() / 384,
        registers: Some(Ratio { used: 1, total: 400 }),
        m20k: Some(Ratio { used: 0, total: m20k }),
        dsp: Some(Ratio { used: 0, total: 2 }),
        placement: vec![((r.x0, r.y0), (r.x1, r.y1))],
    }
}

#[test]
fn the_geometry_sums_whole_tiles_and_never_guesses_a_part_tile() {
    let t = tiles(Rect { x0: 0, y0: 0, x1: 31, y1: 47 }, 16, 24);
    let regions: Vec<RegionUsage> = t.iter().enumerate().map(|(k, &r)| tile_region(k, r, 10 + k as u64)).collect();
    let g = Geometry::of(&regions).expect("a probe's regions");
    assert_eq!(g.tiles.len(), 4);
    let all = g.capacity(Rect { x0: 0, y0: 0, x1: 31, y1: 47 }).unwrap();
    assert_eq!((all.m20k, all.dsp, all.registers, all.alms), (10 + 11 + 12 + 13, 8, 1600, 400));
    assert_eq!(g.capacity(t[1]).unwrap().m20k, 11, "one tile");
    assert_eq!(g.capacity(Rect { x0: 0, y0: 0, x1: 15, y1: 47 }).unwrap().m20k, 10 + 12, "a column of two");
    assert_eq!(g.capacity(Rect { x0: 0, y0: 0, x1: 20, y1: 23 }), None, "a part tile is unmeasured");
    assert_eq!(g.capacity(Rect { x0: 0, y0: 0, x1: 40, y1: 23 }), None, "past the map");
    let mut other = regions.clone();
    other.push(RegionUsage { name: "logos_ch0".into(), ..tile_region(9, t[0], 5) });
    assert_eq!(Geometry::of(&other).unwrap().tiles.len(), 4, "a probe's regions alone");
    let mut uncounted = regions.clone();
    uncounted[2].m20k = None;
    assert!(Geometry::of(&uncounted).is_none(), "a tile without its counts");
}

/// A map of 6 by 6 tiles of 8 by 8 sites, each holding 1,000 ALMs, 4,000 registers, 10 M20Ks and 4 DSP blocks.
fn uniform() -> Geometry {
    let t = tiles(Rect { x0: 0, y0: 0, x1: 47, y1: 47 }, 8, 8);
    Geometry { tiles: t.into_iter().map(|r| (r, Capacity { alms: 1_000, registers: 4_000, m20k: 10, dsp: 4 })).collect() }
}

#[test]
fn a_region_holds_a_unit_when_every_resource_it_needs_fits_its_fill() {
    let c = Capacity { alms: 1_000, registers: 4_000, m20k: 10, dsp: 4 };
    assert!(holds(c, Needs { alms: 800.0, registers: 3_200.0, m20k: 8, dsp: 3 }, Fill::uniform(0.8)));
    assert!(!holds(c, Needs { m20k: 9, ..Needs::default() }, Fill::uniform(0.8)), "9 M20Ks past 0.8 of 10");
    assert!(!holds(c, Needs { registers: 3_300.0, ..Needs::default() }, Fill::uniform(0.8)), "registers alone");
    assert!(holds(c, Needs { dsp: 4, ..Needs::default() }, Fill::uniform(1.0)));
}

#[test]
fn a_regions_logic_and_its_hard_blocks_each_fill_to_their_own_share() {
    let c = Capacity { alms: 1_000, registers: 4_000, m20k: 10, dsp: 4 };
    let fill = Fill { logic: 0.5, m20k: 0.9, dsp: 1.0 };
    assert!(holds(c, Needs { alms: 500.0, registers: 2_000.0, m20k: 9, dsp: 4 }, fill), "half the logic, nine of ten M20Ks, every DSP block");
    assert!(!holds(c, Needs { alms: 501.0, ..Needs::default() }, fill), "past half the ALMs");
    assert!(!holds(c, Needs { registers: 2_001.0, ..Needs::default() }, fill), "past half the registers");
    assert!(!holds(c, Needs { m20k: 10, ..Needs::default() }, fill), "past nine tenths of the M20Ks");
    assert!(!holds(c, Needs { dsp: 5, ..Needs::default() }, fill), "past every DSP block");
    assert_eq!(Fill::uniform(0.7), Fill { logic: 0.7, m20k: 0.7, dsp: 0.7 });
}

#[test]
fn the_candidates_are_the_least_tile_rectangles_that_hold_a_unit_clear_of_what_is_blocked() {
    let g = uniform();
    let needs = Needs { m20k: 35, ..Needs::default() };
    let blocked = [Rect { x0: 0, y0: 0, x1: 15, y1: 15 }];
    let c = candidates(&g, needs, Fill::uniform(1.0), 4.0, &blocked);
    assert!(!c.is_empty());
    for r in &c {
        assert!(holds(g.capacity(*r).unwrap(), needs, Fill::uniform(1.0)), "{r:?} holds the unit");
        assert!(!blocked.iter().any(|b| r.x0 <= b.x1 && b.x0 <= r.x1 && r.y0 <= b.y1 && b.y0 <= r.y1), "{r:?} clear of the block");
        let (w, h) = ((r.x1 - r.x0 + 1) / 8, (r.y1 - r.y0 + 1) / 8);
        assert_eq!(w * h, 4, "{r:?}: four tiles hold 35, three do not");
        assert!(w.max(h) <= 4 * w.min(h), "{r:?}: within the aspect");
    }
    assert!(c.contains(&Rect { x0: 16, y0: 0, x1: 31, y1: 15 }) && c.contains(&Rect { x0: 16, y0: 16, x1: 47, y1: 23 }), "squares and bars alike");
    assert!(!c.contains(&Rect { x0: 16, y0: 16, x1: 47, y1: 31 }), "a rectangle with a smaller one inside it that holds is not least");
    assert!(candidates(&g, Needs { m20k: 361, ..Needs::default() }, Fill::uniform(1.0), 4.0, &[]).is_empty(), "more than the device holds");
}

#[test]
fn the_units_are_placed_in_order_each_where_its_wires_are_shortest() {
    let g = uniform();
    let unit = |name: &str, m20k: u64| Unit { instances: vec![format!("top|{name}")], name: format!("logos_{name}"), needs: Needs { m20k, ..Needs::default() } };
    let units = [unit("hub", 35), unit("store", 75), unit("side", 15)];
    let corners = [(0, 0), (47, 0), (0, 47), (47, 47)];
    let mut wires: Vec<Wire> = corners.iter().map(|&s| Wire::Site(0, s, 512)).collect();
    wires.push(Wire::Units(0, 1, 512));
    wires.push(Wire::Units(1, 2, 512));
    let plan = place(&g, &units, &wires, Fill::uniform(1.0), 4.0, &[]).unwrap();
    assert_eq!(plan.len(), 3);
    assert_eq!(plan[0].rect, Rect { x0: 16, y0: 16, x1: 31, y1: 31 }, "the hub at the centre of its corners");
    for (i, a) in plan.iter().enumerate() {
        assert!(holds(g.capacity(a.rect).unwrap(), units[i].needs, Fill::uniform(1.0)));
        assert_eq!((&a.instances, &a.name), (&units[i].instances, &units[i].name));
        for b in &plan[i + 1..] {
            let r = a.rect;
            assert!(r.x1 < b.rect.x0 || b.rect.x1 < r.x0 || r.y1 < b.rect.y0 || b.rect.y1 < r.y0, "{} and {} apart", a.name, b.name);
        }
    }
    let touching = |a: Rect, b: Rect| (a.x1 + 1 == b.x0 || b.x1 + 1 == a.x0) && a.y0 <= b.y1 && b.y0 <= a.y1
        || (a.y1 + 1 == b.y0 || b.y1 + 1 == a.y0) && a.x0 <= b.x1 && b.x0 <= a.x1;
    assert!(touching(plan[0].rect, plan[1].rect), "the store beside the hub: {:?} {:?}", plan[0].rect, plan[1].rect);
    assert!(touching(plan[1].rect, plan[2].rect), "the side beside the store: {:?} {:?}", plan[1].rect, plan[2].rect);
    assert_eq!(place(&g, &units, &wires, Fill::uniform(1.0), 4.0, &[]).unwrap(), plan, "the same floorplan every time");
    let too_big = [unit("hub", 35), unit("store", 400)];
    assert_eq!(place(&g, &too_big, &wires[..5], Fill::uniform(1.0), 4.0, &[]), Err(PlaceError::NoRegion("top|store".into())));
}

#[test]
fn a_region_is_named_for_its_instance_path() {
    assert_eq!(region_name("ch[0].channel"), "logos_ch0_channel");
    assert_eq!(region_name("engine|fast|unit"), "logos_engine_fast_unit");
    assert_eq!(region_name("engine|fast|lane_port[1].port"), "logos_engine_fast_lane_port1_port");
}

/// The graph of a module `e` is dissolved into: a channel, a hub, a store and a side, and an instance no unit names.
fn wired() -> Graph {
    use crate::codegen_sva::netlist::Link;
    let link = |a: &str, b: &str, bits: u64| Link { a: a.into(), b: b.into(), bits };
    Graph {
        instances: ["ch[0].channel", "e|hub", "e|side", "e|store", "loose"].map(String::from).to_vec(),
        links: vec![
            link("ch[0].channel", "e|hub", 512),
            link("e|hub", "e|side", 64),
            link("e|hub", "e|store", 1024),
            link("e|hub", "loose", 4096),
            link("e|side", "e|store", 512),
        ],
        ports: vec![("ch[0].channel".into(), 1130), ("e|hub".into(), 2)],
    }
}

#[test]
fn a_unit_floorplan_wires_its_units_by_the_graphs_bits_and_each_anchored_unit_to_its_site_by_its_ports() {
    let g = uniform();
    let needs = |m20k: u64| Needs { m20k, ..Needs::default() };
    let units: Vec<Unit> = [("ch[0].channel", 10), ("e|hub", 35), ("e|store", 75), ("e|side", 15)]
        .iter()
        .map(|&(p, m)| Unit { instances: vec![p.to_string()], name: region_name(p), needs: needs(m) })
        .collect();
    let anchors = [
        Anchor { instance: "top|shell|ch[0].channel".into(), clock: "bank 3".into(), site: (47, 47) },
        Anchor { instance: "top|shell|e|elsewhere".into(), clock: "bank 0".into(), site: (0, 0) },
    ];
    let plan = unit_floorplan(&g, &wired(), "top|shell", &units, &anchors, Fill::uniform(1.0), 4.0, &[]).unwrap();
    let spelled: Vec<Unit> = units.iter().map(|u| Unit { instances: vec![format!("top|shell|{}", u.instances[0])], name: u.name.clone(), needs: u.needs }).collect();
    let wires = [Wire::Units(0, 1, 512), Wire::Units(1, 3, 64), Wire::Units(1, 2, 1024), Wire::Units(3, 2, 512), Wire::Site(0, (47, 47), 1130)];
    assert_eq!(plan, place(&g, &spelled, &wires, Fill::uniform(1.0), 4.0, &[]).unwrap(), "the graph's wires among the units, the anchor's by the ports' bits");
    assert_eq!((plan[0].instances[0].as_str(), plan[0].name.as_str()), ("top|shell|ch[0].channel", "logos_ch0_channel"));
    assert_eq!(plan[0].rect.distance((47, 47)), 0, "the channel on its anchor: {:?}", plan[0].rect);
    let bare = unit_floorplan(&g, &wired(), "", &units[1..2], &anchors, Fill::uniform(1.0), 4.0, &[]).unwrap();
    assert_eq!(bare[0].instances, ["e|hub"], "under no module, the path alone");
    let blocked = [Rect { x0: 40, y0: 40, x1: 47, y1: 47 }];
    let moved = unit_floorplan(&g, &wired(), "top|shell", &units[..1], &anchors, Fill::uniform(1.0), 4.0, &blocked).unwrap();
    // The two tiles beside the blocked corner lie as near the anchor: the one of the lower top row.
    assert_eq!(moved[0].rect, Rect { x0: 40, y0: 32, x1: 47, y1: 39 }, "beside the blocked corner");
}

#[test]
fn a_unit_floorplan_refuses_a_unit_the_module_lacks_and_two_units_of_one_name() {
    let g = uniform();
    let small = Needs { m20k: 1, ..Needs::default() };
    let one = |path: &str| Unit { instances: vec![path.to_string()], name: region_name(path), needs: small };
    let absent = [one("e|hub"), one("e|absent")];
    assert_eq!(unit_floorplan(&g, &wired(), "", &absent, &[], Fill::uniform(1.0), 4.0, &[]), Err(PlaceError::Unknown("e|absent".into())));
    let graph = Graph { instances: vec!["a.b".into(), "a|b".into()], links: Vec::new(), ports: Vec::new() };
    let twins = [Unit { name: "logos_x".into(), ..one("a|b") }, Unit { name: "logos_x".into(), ..one("a.b") }];
    assert_eq!(unit_floorplan(&g, &graph, "", &twins, &[], Fill::uniform(1.0), 4.0, &[]), Err(PlaceError::SameName("a|b".into(), "a.b".into())));
}

fn ram(kind: &str, ports: &[(u64, u64)], byte: Option<u64>, old_data: bool) -> Ram {
    Ram { name: "top|m".into(), kind: kind.into(), ports: ports.to_vec(), byte, old_data }
}

#[test]
fn a_memory_takes_the_m20k_blocks_the_fitter_gives_it() {
    // hybrid7b's store: 32,064 words of 512 bits with 8-bit byte enables, a stack of 32,768 words, 32 bits a block.
    assert_eq!(ram("AUTO", &[(32_064, 512), (32_064, 512)], Some(8), true).m20k(), 1_024);
    // A row of the vector unit's registers, 1,088 words: a stack of 2,048.
    assert_eq!(ram("AUTO", &[(1_088, 512), (1_088, 512)], Some(8), true).m20k(), 64);
    assert_eq!(ram("AUTO", &[(1_088, 512), (1_088, 512)], None, true).m20k(), 52, "no byte enables: 40 bits a block");
    assert_eq!(ram("AUTO", &[(1_088, 512)], Some(10), true).m20k(), 52, "10-bit bytes fill a lane");
    assert_eq!(ram("AUTO", &[(16, 32), (16, 32)], None, false).m20k(), 0, "small enough for MLABs");
    assert_eq!(ram("AUTO", &[(16, 32), (16, 32)], None, true).m20k(), 1, "old data across its ports, which MLABs cannot give");
    assert_eq!(ram("AUTO", &[(32, 42)], None, false).m20k(), 0);
    assert_eq!(ram("AUTO", &[(32, 43)], None, false).m20k(), 2, "past an MLAB's width, two blocks of 40 bits");
    assert_eq!(ram("AUTO", &[(64, 10)], None, false).m20k(), 0);
    assert_eq!(ram("AUTO", &[(64, 11)], None, false).m20k(), 1);
    assert_eq!(ram("AUTO", &[(128, 5)], None, false).m20k(), 0);
    assert_eq!(ram("AUTO", &[(128, 6)], None, false).m20k(), 1);
    assert_eq!(ram("AUTO", &[(16, 32), (256, 2)], None, false).m20k(), 1, "every port small, or none of it in MLABs");
    assert_eq!(ram("MLAB", &[(1_024, 32)], None, false).m20k(), 0, "asked into MLABs");
    assert_eq!(ram("M20K block", &[(16, 12), (16, 12)], None, false).m20k(), 1, "asked into an M20K, never an MLAB");
    // 512 words of 512 bits are 13 blocks, 16,384 words of 16 bits 16: the costlier port.
    assert_eq!(ram("M20K block", &[(512, 512), (16_384, 16)], None, false).m20k(), 16);
}

#[test]
fn the_memories_are_the_ram_summarys_each_with_its_inferred_parameters() {
    let syn = "\
+------------------------------------------------------------------+
; Synthesis RAM Summary for Partition \"root_partition\"           ;
+------------------------------------------------------------------+
; Name ; Type ; Mode ; Port A Depth ; Port A Width ; Port B Depth ; Port B Width ; Size ; MIF ;
+------------------------------------------------------------------+
; top|s|mem_rtl_0|auto_generated|altera_syncram_impl1|ALTERA_SYNCRAM ; AUTO ; Simple Dual Port ; 32064 ; 512 ; 32064 ; 512 ; 16416768 ; None ;
; top|s|mem_rtl_00|auto_generated|ALTERA_SYNCRAM ; AUTO ; Simple Dual Port ; 16 ; 32 ; 16 ; 32 ; 512 ; None ;
; top|f|fifo|ram ; M20K block ; Simple Dual Port ; 512 ; 512 ; 512 ; 512 ; 262,144 ; None ;
; top|r|ram ; MLAB ; Single Port ; 32 ; 20 ;  ;  ; 640 ; None ;
+------------------------------------------------------------------+


+------------------------------------------------------------------+
; Parameter Settings for Inferred Entity Instance: top|s|mem_rtl_0 ;
+------------------------------------------------------------------+
; Parameter Name                     ; Value    ; Type           ;
+------------------------------------------------------------------+
; BYTE_SIZE                          ; 8        ; Untyped        ;
; READ_DURING_WRITE_MODE_MIXED_PORTS ; OLD_DATA ; Untyped        ;
; WIDTH_BYTEENA_A                    ; 64       ; Signed Integer ;
; WIDTH_BYTEENA_B                    ; 1        ; Signed Integer ;
+------------------------------------------------------------------+
";
    let ram = |name: &str, kind: &str, ports: &[(u64, u64)], byte, old_data| Ram { name: name.into(), kind: kind.into(), ports: ports.to_vec(), byte, old_data };
    assert_eq!(
        synthesized_rams(syn),
        vec![
            ram("top|s|mem_rtl_0|auto_generated|altera_syncram_impl1|ALTERA_SYNCRAM", "AUTO", &[(32_064, 512), (32_064, 512)], Some(8), true),
            ram("top|s|mem_rtl_00|auto_generated|ALTERA_SYNCRAM", "AUTO", &[(16, 32), (16, 32)], None, false),
            ram("top|f|fifo|ram", "M20K block", &[(512, 512), (512, 512)], None, false),
            ram("top|r|ram", "MLAB", &[(32, 20)], None, false),
        ],
        "a section is the memory's whose name continues its path at a level, a single port its first alone"
    );
    assert_eq!(synthesized_rams(""), Vec::new());
}

fn sharing_an_edge(a: &Rect, b: &Rect) -> bool {
    ((a.x1 + 1 == b.x0 || b.x1 + 1 == a.x0) && a.y0 <= b.y1 && b.y0 <= a.y1) || ((a.y1 + 1 == b.y0 || b.y1 + 1 == a.y0) && a.x0 <= b.x1 && b.x0 <= a.x1)
}

/// Whether every tile reaches every other through tiles that share an edge.
fn connected(tiles: &[Rect]) -> bool {
    let mut reached = vec![false; tiles.len()];
    let mut stack = vec![0];
    reached[0] = true;
    while let Some(a) = stack.pop() {
        for b in 0..tiles.len() {
            if !reached[b] && sharing_an_edge(&tiles[a], &tiles[b]) {
                reached[b] = true;
                stack.push(b);
            }
        }
    }
    reached.iter().all(|r| *r)
}

fn total(g: &Geometry, tiles: &[Rect]) -> Capacity {
    tiles.iter().fold(Capacity::default(), |c, t| {
        let h = g.capacity(*t).unwrap();
        Capacity { alms: c.alms + h.alms, registers: c.registers + h.registers, m20k: c.m20k + h.m20k, dsp: c.dsp + h.dsp }
    })
}

#[test]
fn a_grown_region_is_the_tiles_nearest_its_seed_where_its_wires_are_shortest() {
    let g = uniform();
    let unit = |name: &str, m20k: u64| Unit { instances: vec![format!("top|{name}")], name: format!("logos_{name}"), needs: Needs { m20k, ..Needs::default() } };
    let units = [unit("hub", 35), unit("store", 75), unit("side", 15)];
    let mut wires: Vec<Wire> = [(0, 0), (47, 0), (0, 47), (47, 47)].iter().map(|&s| Wire::Site(0, s, 512)).collect();
    wires.push(Wire::Units(0, 1, 512));
    wires.push(Wire::Units(1, 2, 512));
    let plan = grow(&g, &units, &wires, Fill::uniform(1.0), &[]).unwrap();
    assert_eq!(plan.len(), 3);
    assert_eq!(plan[0].rects(), vec![Rect { x0: 16, y0: 16, x1: 31, y1: 31 }], "the hub's four tiles at the centre of its corners");
    for (i, r) in plan.iter().enumerate() {
        assert_eq!((&r.instances, &r.name), (&units[i].instances, &units[i].name));
        assert!(holds(total(&g, &r.tiles), units[i].needs, Fill::uniform(1.0)), "{} holds its unit", r.name);
        assert!(connected(&r.tiles), "{}: one piece: {:?}", r.name, r.tiles);
        let mut sorted = r.tiles.clone();
        sorted.sort_by_key(|t| (t.y0, t.x0));
        assert_eq!(sorted, r.tiles, "{}: in the map's order", r.name);
        for other in &plan[i + 1..] {
            assert!(r.tiles.iter().all(|t| !other.tiles.contains(t)), "{} and {} share no tile", r.name, other.name);
        }
    }
    let touching = |a: &Region, b: &Region| a.tiles.iter().any(|t| b.tiles.iter().any(|u| sharing_an_edge(t, u)));
    assert!(touching(&plan[0], &plan[1]), "the store beside the hub");
    assert!(touching(&plan[1], &plan[2]), "the side beside the store");
    assert_eq!(plan[1].tiles.len(), 8, "75 M20Ks in eight tiles of ten, no more");
    assert_eq!(grow(&g, &units, &wires, Fill::uniform(1.0), &[]).unwrap(), plan, "the same floorplan every time");
    let too_big = [unit("hub", 35), unit("store", 400)];
    assert_eq!(grow(&g, &too_big, &wires[..5], Fill::uniform(1.0), &[]), Err(PlaceError::NoRegion("top|store".into())));
}

#[test]
fn a_region_grows_round_what_no_rectangle_clears() {
    let g = uniform();
    // The middle four by four tiles blocked: a ring of twenty tiles, one tile wide, is left.
    let block = [Rect { x0: 8, y0: 8, x1: 39, y1: 39 }];
    let ring = [Unit { instances: vec!["top|ring".into()], name: "logos_ring".into(), needs: Needs { m20k: 95, ..Needs::default() } }];
    let wires = [Wire::Site(0, (0, 0), 1)];
    assert_eq!(place(&g, &ring, &wires, Fill::uniform(1.0), 4.0, &block), Err(PlaceError::NoRegion("top|ring".into())), "no rectangle of ten tiles in the ring");
    let plan = grow(&g, &ring, &wires, Fill::uniform(1.0), &block).unwrap();
    let tiles = &plan[0].tiles;
    assert_eq!(tiles.len(), 10, "95 M20Ks in ten tiles of ten");
    assert!(connected(tiles) && tiles.iter().all(|t| !(t.x0 <= 39 && 8 <= t.x1 && t.y0 <= 39 && 8 <= t.y1)), "one piece of the ring: {tiles:?}");
    // From the corner its wire pulls to, the tiles nearest it: six along the top, four down the side.
    assert_eq!(plan[0].rects(), vec![Rect { x0: 0, y0: 0, x1: 47, y1: 7 }, Rect { x0: 0, y0: 8, x1: 7, y1: 39 }]);
    let small = [Unit { needs: Needs { m20k: 201, ..Needs::default() }, ..ring[0].clone() }];
    assert_eq!(grow(&g, &small, &wires, Fill::uniform(1.0), &block), Err(PlaceError::NoRegion("top|ring".into())), "more than the ring holds");
}

#[test]
fn a_region_merges_its_tiles_into_rectangles_and_its_settings_name_them_in_one_place_region() {
    let t = |x0: u32, y0: u32| Rect { x0, y0, x1: x0 + 7, y1: y0 + 7 };
    let column = Region { instances: vec!["top|c".into()], name: "logos_c".into(), tiles: vec![t(0, 0), t(0, 8), t(0, 16)] };
    assert_eq!(column.rects(), vec![Rect { x0: 0, y0: 0, x1: 7, y1: 23 }], "a column of tiles is one rectangle");
    let l = Region { instances: vec!["top|u".into()], name: "logos_u".into(), tiles: vec![t(0, 0), t(8, 0), t(0, 8)] };
    assert_eq!(l.rects(), vec![Rect { x0: 0, y0: 0, x1: 15, y1: 7 }, Rect { x0: 0, y0: 8, x1: 7, y1: 15 }]);
    let gap = Region { instances: vec!["top|g".into()], name: "logos_g".into(), tiles: vec![t(0, 0), t(16, 0)] };
    assert_eq!(gap.rects().len(), 2, "tiles apart are not one run");
    assert_eq!(
        regions_tcl(&[l]),
        "set_instance_assignment -name PLACE_REGION \"X0 Y0 X15 Y7;X0 Y8 X7 Y15\" -to top|u\n\
         set_instance_assignment -name REGION_NAME logos_u -to top|u\n\
         set_instance_assignment -name CORE_ONLY_PLACE_REGION ON -to top|u\n\
         set_instance_assignment -name RESERVE_PLACE_REGION OFF -to top|u\n"
    );
}

#[test]
fn unit_regions_grow_the_units_on_the_graphs_wires() {
    let g = uniform();
    let needs = |m20k: u64| Needs { m20k, ..Needs::default() };
    let units: Vec<Unit> = [("ch[0].channel", 10), ("e|hub", 35), ("e|store", 75), ("e|side", 15)]
        .iter()
        .map(|&(p, m)| Unit { instances: vec![p.to_string()], name: region_name(p), needs: needs(m) })
        .collect();
    let anchors = [Anchor { instance: "top|shell|ch[0].channel".into(), clock: "bank 3".into(), site: (47, 47) }];
    let regions = unit_regions(&g, &wired(), "top|shell", &units, &anchors, Fill::uniform(1.0), &[]).unwrap();
    let (placing, wires) = unit_wires(&wired(), "top|shell", &units, &anchors).unwrap();
    assert_eq!(regions, grow(&g, &placing, &wires, Fill::uniform(1.0), &[]).unwrap());
    assert_eq!(regions[0].tiles, vec![Rect { x0: 40, y0: 40, x1: 47, y1: 47 }], "the channel on its anchor's tile");
    let absent = [Unit { instances: vec!["e|absent".into()], name: "logos_absent".into(), needs: needs(1) }];
    assert_eq!(unit_regions(&g, &wired(), "", &absent, &[], Fill::uniform(1.0), &[]), Err(PlaceError::Unknown("e|absent".into())));
}

/// A unit's memories are those under its own path: `top|s` counts none of `top|sx`'s, and a memory byte-enabled on its
/// second port alone is byte-enabled.
#[test]
fn a_units_memories_are_under_its_own_path_and_either_port_enables_its_bytes() {
    let syn = "\
+------------------------------------------------------------------+
; Partition \"root_partition\" Resource Utilization by Entity       ;
+------------------------------------------------------------------+
; Compilation Hierarchy Node ; Combinational ALUTs ; Dedicated Logic Registers ; DSP Blocks ; Full Hierarchy Name ;
+------------------------------------------------------------------+
; |s|                        ; 10 (10)             ; 20 (20)                   ; 0 (0)      ; top|s               ;
; |sx|                       ; 10 (10)             ; 20 (20)                   ; 0 (0)      ; top|sx              ;
+------------------------------------------------------------------+


+------------------------------------------------------------------+
; Synthesis RAM Summary for Partition \"root_partition\"           ;
+------------------------------------------------------------------+
; Name ; Type ; Mode ; Port A Depth ; Port A Width ; Port B Depth ; Port B Width ; Size ; MIF ;
+------------------------------------------------------------------+
; top|s|mem_rtl_0|auto_generated|ALTERA_SYNCRAM ; M20K block ; Simple Dual Port ; 512 ; 512 ; 512 ; 512 ; 262144 ; None ;
; top|sx|mem_rtl_0|auto_generated|ALTERA_SYNCRAM ; M20K block ; Simple Dual Port ; 512 ; 512 ; 512 ; 512 ; 262144 ; None ;
+------------------------------------------------------------------+


+------------------------------------------------------------------+
; Parameter Settings for Inferred Entity Instance: top|sx|mem_rtl_0 ;
+------------------------------------------------------------------+
; Parameter Name  ; Value ; Type           ;
+------------------------------------------------------------------+
; BYTE_SIZE       ; 8     ; Untyped        ;
; WIDTH_BYTEENA_A ; 1     ; Signed Integer ;
; WIDTH_BYTEENA_B ; 64    ; Signed Integer ;
+------------------------------------------------------------------+
";
    let packing = crate::codegen_sva::preflight::Packing { per_alut: 1.0, per_register: None };
    assert_eq!(synthesized_needs(syn, &["top|s"], &packing).map(|n| n.m20k), Ok(13), "its own 512 x 512, not top|sx's too");
    assert_eq!(synthesized_rams(syn)[1].byte, Some(8), "byte-enabled on its second port");
    assert_eq!(synthesized_needs(syn, &["top|sx"], &packing).map(|n| n.m20k), Ok(16), "16 blocks of 32 bits");
}

/// An `AUTO` memory goes into MLABs only within each shape's depth: 32 words, 64, 128, a word more each an M20K.
#[test]
fn the_mlab_shapes_end_at_their_depths() {
    assert_eq!(ram("AUTO", &[(32, 40)], None, false).m20k(), 0);
    assert_eq!(ram("AUTO", &[(33, 40)], None, false).m20k(), 1, "33 words of 40 bits");
    assert_eq!(ram("AUTO", &[(64, 10)], None, false).m20k(), 0);
    assert_eq!(ram("AUTO", &[(65, 10)], None, false).m20k(), 1, "65 words of 10 bits");
    assert_eq!(ram("AUTO", &[(128, 5)], None, false).m20k(), 0);
    assert_eq!(ram("AUTO", &[(129, 5)], None, false).m20k(), 1, "129 words of 5 bits");
}

/// The memories are every partition's: a flat image's fabric partition holds memories of its own.
#[test]
fn the_memories_are_every_partitions() {
    let table = |partition: &str, row: &str| {
        format!(
            "+----+\n; Synthesis RAM Summary for Partition \"{partition}\" ;\n+----+\n\
             ; Name ; Type ; Mode ; Port A Depth ; Port A Width ; Port B Depth ; Port B Width ; Size ; MIF ;\n+----+\n{row}\n+----+\n\n\n"
        )
    };
    let syn = table("root_partition", "; top|a|ram ; M20K block ; Simple Dual Port ; 512 ; 40 ; 512 ; 40 ; 20480 ; None ;")
        + &table("auto_fab_0", "; fab|b|ram ; M20K block ; Simple Dual Port ; 512 ; 80 ; 512 ; 80 ; 40960 ; None ;");
    let rams = synthesized_rams(&syn);
    assert_eq!(rams.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["top|a|ram", "fab|b|ram"]);
    assert_eq!(rams.iter().map(Ram::m20k).sum::<u64>(), 1 + 2);
}

/// The tiles a probe's fit found no flip-flop in are the regions its log refuses for one, each once, in its order.
#[test]
fn the_tiles_a_probe_found_no_flip_flop_in_are_the_regions_its_log_refuses() {
    let log = "\
Info (22300): Design uses Placement Effort Multiplier = 1.0.
Error (21728): Deterministic overuse of FF resources in region (160,0) to (175,23)
    Info (22480): The region requires 1 FF resources but there are only 0 FF resources available
    Info (22479): Logic Lock Region geo_10: (160,0) to (175,23)
Error (21728): Deterministic overuse of FF resources in region (64,432) to (79,432)
    Info (22480): The region requires 1 FF resources but there are only 0 FF resources available
Error (21728): Deterministic overuse of FF resources in region (160,0) to (175,23)
Error (12345): Deterministic overuse of M20K resources in region (0,0) to (15,23)
Error: An error occurred during placement
";
    assert_eq!(fabricless(log), vec![Rect { x0: 160, y0: 0, x1: 175, y1: 23 }, Rect { x0: 64, y0: 432, x1: 79, y1: 432 }]);
    assert_eq!(fabricless("Info (20274): Successfully committed placed database.\n"), Vec::new(), "a fit with none");
}

/// A region of several instances assigns each of them to it: one place region, one name, every member.
#[test]
fn a_region_of_several_instances_assigns_each_of_them_to_it() {
    let members = vec!["top|unit|chan[0].feed".to_string(), "top|unit|chan[0].walk".to_string()];
    let text = tcl(&[Assignment { instances: members.clone(), name: "logos_chan0".into(), rect: Rect { x0: 1, y0: 2, x1: 3, y1: 4 } }]);
    assert_eq!(text.lines().count(), 2 * 4);
    let t = |x0: u32, y0: u32| Rect { x0, y0, x1: x0 + 7, y1: y0 + 7 };
    let grown = regions_tcl(&[Region { instances: members.clone(), name: "logos_chan0".into(), tiles: vec![t(0, 0), t(8, 0)] }]);
    for m in &members {
        assert!(text.contains(&format!("set_instance_assignment -name PLACE_REGION \"X1 Y2 X3 Y4\" -to {m}\n")), "{text}");
        assert!(text.contains(&format!("set_instance_assignment -name REGION_NAME logos_chan0 -to {m}\n")), "{text}");
        assert!(grown.contains(&format!("set_instance_assignment -name PLACE_REGION \"X0 Y0 X15 Y7\" -to {m}\n")), "{grown}");
        assert_eq!(grown.matches(&format!("-to {m}\n")).count(), 4, "{m}: its four settings");
    }
}

/// A unit of several instances takes each one's wires to the other units, none between its own, and each one's anchor;
/// a unit of none, and an instance two units hold, are refused by name.
#[test]
fn a_unit_of_several_instances_takes_the_wires_of_each_and_none_between_them() {
    let small = Needs { m20k: 1, ..Needs::default() };
    let unit = |name: &str, paths: &[&str]| Unit { instances: paths.iter().map(|p| p.to_string()).collect(), name: name.into(), needs: small };
    let units = [unit("logos_front", &["ch[0].channel", "e|hub"]), unit("logos_back", &["e|side", "e|store"])];
    let anchors = [Anchor { instance: "top|ch[0].channel".into(), clock: "bank 3".into(), site: (47, 47) }];
    let (placing, wires) = unit_wires(&wired(), "top", &units, &anchors).unwrap();
    assert_eq!(placing[1].instances, ["top|e|side", "top|e|store"]);
    assert_eq!(wires, vec![Wire::Units(0, 1, 64), Wire::Units(0, 1, 1024), Wire::Site(0, (47, 47), 1130)], "the channel's 512 to the hub and the side's 512 to the store stay inside");
    assert_eq!(unit_wires(&wired(), "", &[unit("logos_none", &[])], &[]), Err(PlaceError::Empty("logos_none".into())));
    assert_eq!(unit_wires(&wired(), "", &[unit("logos_a", &["e|hub"]), unit("logos_b", &["e|side", "e|hub"])], &[]), Err(PlaceError::Twice("e|hub".into())));
    assert_eq!(unit_wires(&wired(), "", &[unit("logos_a", &["e|hub", "e|hub"])], &[]), Err(PlaceError::Twice("e|hub".into())), "twice in one unit");
    let g = uniform();
    let regions = unit_regions(&g, &wired(), "top", &units, &anchors, Fill::uniform(1.0), &[]).unwrap();
    assert_eq!(regions[0].instances, ["top|ch[0].channel", "top|e|hub"]);
    assert_eq!(regions[0].tiles, vec![Rect { x0: 40, y0: 40, x1: 47, y1: 47 }], "the front on the channel's anchor");
}

/// A group is wired to the anchor of whichever member it anchors, by that member's own ports, not its first member's
/// alone.
#[test]
fn a_group_is_wired_to_the_anchor_of_each_of_its_members() {
    let small = Needs { m20k: 1, ..Needs::default() };
    let units = [Unit { instances: vec!["e|hub".into(), "ch[0].channel".into()], name: "logos_front".into(), needs: small }];
    let anchors = [
        Anchor { instance: "top|ch[0].channel".into(), clock: "bank 3".into(), site: (47, 47) },
        Anchor { instance: "top|e|hub".into(), clock: "bank 1".into(), site: (0, 47) },
    ];
    let (_, wires) = unit_wires(&wired(), "top", &units, &anchors).unwrap();
    assert_eq!(wires, vec![Wire::Site(0, (0, 47), 2), Wire::Site(0, (47, 47), 1130)], "the hub's two bits to its bank, the channel's 1,130 to its");
}

/// A group's needs are its members' ALUTs, registers, DSP blocks and memories summed and packed once: the ALMs one
/// region of them takes, the fitter packing one member's registers beside another's LUTs, not each member's ALMs added.
#[test]
fn a_groups_needs_pack_its_members_together() {
    let syn = "\
+------------------------------------------------------------------+
; Partition \"root_partition\" Resource Utilization by Entity       ;
+------------------------------------------------------------------+
; Compilation Hierarchy Node ; Combinational ALUTs ; Dedicated Logic Registers ; DSP Blocks ; Full Hierarchy Name ;
+------------------------------------------------------------------+
; |l|                        ; 100 (100)           ; 40 (40)                   ; 1 (1)      ; top|l               ;
; |r|                        ; 0 (0)               ; 100 (100)                 ; 2 (2)      ; top|r               ;
+------------------------------------------------------------------+


+------------------------------------------------------------------+
; Synthesis RAM Summary for Partition \"root_partition\"           ;
+------------------------------------------------------------------+
; Name ; Type ; Mode ; Port A Depth ; Port A Width ; Port B Depth ; Port B Width ; Size ; MIF ;
+------------------------------------------------------------------+
; top|l|mem_rtl_0|auto_generated|ALTERA_SYNCRAM ; M20K block ; Simple Dual Port ; 512 ; 40 ; 512 ; 40 ; 20480 ; None ;
; top|r|mem_rtl_0|auto_generated|ALTERA_SYNCRAM ; M20K block ; Simple Dual Port ; 512 ; 80 ; 512 ; 80 ; 40960 ; None ;
+------------------------------------------------------------------+
";
    let packing = crate::codegen_sva::preflight::Packing { per_alut: 0.5, per_register: Some(0.25) };
    assert_eq!(synthesized_needs(syn, &["top|l"], &packing), Ok(Needs { alms: 50.0, registers: 40.0, m20k: 1, dsp: 1 }));
    assert_eq!(synthesized_needs(syn, &["top|r"], &packing), Ok(Needs { alms: 25.0, registers: 100.0, m20k: 2, dsp: 2 }));
    assert_eq!(
        synthesized_needs(syn, &["top|l", "top|r"], &packing),
        Ok(Needs { alms: 50.0, registers: 140.0, m20k: 3, dsp: 3 }),
        "the larger of 100 ALUTs x 0.5 and 140 registers x 0.25, not 50 + 25"
    );
    assert_eq!(synthesized_needs(syn, &["top|l", "top|x"], &packing), Err("top|x".to_string()), "a member the entity table lacks, by name");
    assert_eq!(synthesized_needs::<&str>(syn, &[], &packing), Ok(Needs::default()), "a group of none needs nothing");
}

/// A group with a member the module lacks is refused by that member, wherever it stands in the group.
#[test]
fn a_group_is_refused_by_any_member_the_module_lacks() {
    let small = Needs { m20k: 1, ..Needs::default() };
    let units = [Unit { instances: vec!["e|hub".into(), "e|absent".into()], name: "logos_front".into(), needs: small }];
    assert_eq!(unit_wires(&wired(), "", &units, &[]), Err(PlaceError::Unknown("e|absent".into())));
}

/// The instances no unit holds, each with the bits it carries to the units, heaviest first: a path between units
/// through one of them is no wire of the floorplan's until its module is dissolved.
#[test]
fn the_loose_instances_are_named_by_the_bits_they_carry_to_the_units() {
    let small = Needs { m20k: 1, ..Needs::default() };
    let unit = |paths: &[&str]| Unit { instances: paths.iter().map(|p| p.to_string()).collect(), name: region_name(paths[0]), needs: small };
    let units = [unit(&["ch[0].channel"]), unit(&["e|hub"]), unit(&["e|store"])];
    assert_eq!(loose(&wired(), &units), vec![("loose".to_string(), 4096), ("e|side".to_string(), 64 + 512)]);
    let all = [unit(&["ch[0].channel", "e|hub"]), unit(&["e|side", "e|store", "loose"])];
    assert_eq!(loose(&wired(), &all), Vec::new(), "every instance held");
    assert_eq!(loose(&wired(), &[unit(&["e|side"])]), vec![("e|store".to_string(), 512), ("e|hub".to_string(), 64)], "a loose instance's bits to the units alone");
}

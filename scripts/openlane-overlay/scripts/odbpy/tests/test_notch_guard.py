import importlib.util, os, sys, types

ODBPY = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
# Stub the odb/click/reader imports so the pure-geometry helpers can be unit-tested.
for m in ("odb", "click", "reader"):
    sys.modules.setdefault(m, types.ModuleType(m))
sys.modules["click"].command = lambda *a, **k: (lambda f: f)
sys.modules["click"].option = lambda *a, **k: (lambda f: f)
sys.modules["reader"].click_odb = lambda f: f
spec = importlib.util.spec_from_file_location(
    "jog", os.path.join(ODBPY, "sat_antenna_jog.py"))
jog = importlib.util.module_from_spec(spec)
spec.loader.exec_module(jog)

SP = 140   # sky130 met1 spacing (m1.2), DBU

bad = 0
def check(name, got, want):
    global bad
    bad += got != want
    print(f"  {'ok ' if got == want else 'FAIL'} {name:66} -> {got} (want {want})")

# MEASURED: infinichip net308 (RUN_2026.09.23_09.31.39). The jog bridged at (7930, 191250); the
# pin's L1M1 access via sits at (7590, 191250). Their met1 enclosures, both on net308:
l1m1 = (7445, 191135, 7735, 191365)     # L1M1_PR met1 box, 290 x 230
m1m2 = (7770, 191120, 8090, 191380)     # M1M2_PR met1 box, 320 x 260
# The golden sky130A_mr deck reported m1.2 on exactly this edge pair,
# (7.77,191.12;7.77,191.18)/(7.735,191.18;7.735,191.135): a 35 nm same-net notch. Magic passed it.
check("gap between the measured enclosures is 35 DBU", jog.rect_gap(l1m1, m1m2), 35)
check("measured bridge enclosure makes a same-net notch", jog.makes_notch([m1m2], [l1m1], SP), True)
moved = (8110, 191120, 8430, 191380)     # the same via one scan step (340) further along
check("one scan step further clears the spacing rule", jog.makes_notch([moved], [l1m1], SP), False)
check("overlapping same-net shapes MERGE (no notch)",
      jog.makes_notch([(7700, 191120, 8020, 191380)], [l1m1], SP), False)
check("abutting shapes merge (gap 0)", jog.makes_notch([(7735, 191120, 8055, 191380)], [l1m1], SP), False)
check("exactly the spacing is legal", jog.makes_notch([(7875, 191120, 8195, 191380)], [l1m1], SP), False)
check("corner-to-corner distance is Euclidean (100,100 -> 141 >= 140)",
      jog.makes_notch([(7835, 191465, 8155, 191725)], [l1m1], SP), False)
check("corner-to-corner under spacing (60,60 -> 85 < 140) is a notch",
      jog.makes_notch([(7795, 191425, 8115, 191685)], [l1m1], SP), True)

# NECKS. Same-net metal that overlaps merges into ONE polygon, and the merged polygon has to clear the
# WIDTH rule: two shapes that overlap (or touch) only at a corner, or share only a short stretch of edge,
# leave a neck the deck measures across. MEASURED: gemm_mac_realbist net _06199_ (RUN_2026.10.03_08.33.45)
# — a met3->met4 bridge at (106940, 342380) put its M3M4 met4 enclosure over the net's own met4 via
# enclosure 215 nm away on both axes. They overlapped in a 75 x 75 nm corner square; golden sky130A_mr
# reported m4.1 (min width 0.3 um) twice on the two edge pairs of that 106 nm neck. Magic read 0.
W4 = 300   # sky130 met4 width (m4.1), DBU
own_m4 = (106560, 342000, 106850, 342290)      # the net's own met4 enclosure, 290 x 290
bridge_m4 = (106775, 342215, 107065, 342505)   # the bridge via's met4 enclosure, 290 x 290
check("measured corner overlap is no notch (they overlap)", jog.makes_notch([bridge_m4], [own_m4], 300), False)
check("measured corner overlap is a 106 nm neck", jog.makes_neck([bridge_m4], [own_m4], W4), True)
check("a corner overlap whose diagonal reaches the width is no neck (215,215 -> 304 >= 300)",
      jog.makes_neck([(106635, 342075, 106925, 342365)], [own_m4], W4), False)
check("the neck is measured Euclidean across the overlap (200,200 -> 283 < 300)",
      jog.makes_neck([(106650, 342090, 106940, 342380)], [own_m4], W4), True)
check("a neck exactly the width is legal (180,240 -> 300)",
      jog.makes_neck([(106670, 342050, 106960, 342340)], [own_m4], W4), False)
check("an overlap spanning the other's height is a step, not a neck",
      jog.makes_neck([(106775, 342000, 107065, 342290)], [own_m4], W4), False)
check("corners touching at one point are a zero-width neck",
      jog.makes_neck([(106850, 342290, 107140, 342580)], [own_m4], W4), True)
check("abutting along 50 nm of edge is a 50 nm neck",
      jog.makes_neck([(106850, 342240, 107140, 342530)], [own_m4], W4), True)
check("abutting along a whole side merges cleanly",
      jog.makes_neck([(106850, 342000, 107140, 342290)], [own_m4], W4), False)
check("a neck buried in the net's own solid metal is no neck",
      jog.makes_neck([bridge_m4], [own_m4, (106500, 341950, 107100, 342550)], W4), False)
check("a shape spanning the other's width merges as a step, however little it overlaps",
      jog.makes_neck([(109950, 342250, 110250, 342550)], [(110000, 342000, 110200, 342300)], W4), False)
check("shapes apart on both axes are the notch guard's case, not a neck (100,100 -> 141)",
      jog.makes_neck([(106950, 342390, 107240, 342680)], [own_m4], W4), False)
check("disjoint shapes are the notch guard's case, not a neck",
      jog.makes_neck([(108000, 342000, 108290, 342290)], [own_m4], W4), False)
check("two NEW shapes necking each other count too",
      jog.makes_neck([own_m4, bridge_m4], [], W4), True)
check("two OWN shapes necking each other are not the bridge's doing",
      jog.makes_neck([(200000, 200000, 200290, 200290)], [own_m4, bridge_m4], W4), False)

# SELF-NOTCH. MEASURED: fabric_horner_seq net _07269_ (RUN_2026.09.23_12.05.48) — a 340 nm bridge
# window put M1M2 vias at x=330890 and 331230. Their met1 enclosures (320 wide) stood 20 nm apart
# once the met1 between them was lifted, and their met2 enclosures (260 tall, over a 140 nm bridge
# wire) left an 80 nm notch above and below it: golden sky130A_mr m1.2 x1 + m2.2 x2, Magic silent.
e0_m1, e1_m1 = (330730, 555940, 331050, 556200), (331070, 555940, 331390, 556200)
e0_m2, e1_m2 = (330760, 555910, 331020, 556230), (331100, 555910, 331360, 556230)
check("the bridge's own two met1 enclosures notch each other (20 nm)",
      jog.self_notch([e0_m1, e1_m1], SP), True)
check("the bridge's own two met2 enclosures notch each other (80 nm)",
      jog.self_notch([e0_m2, e1_m2], SP), True)
w500 = [(330730, 555940, 331050, 556200), (331230, 555940, 331550, 556200)]
check("a 500 nm window leaves 180 nm between them (legal)", jog.self_notch(w500, SP), False)

# WINDOWS FROM THE VIA. Two enclosures one window apart must clear the spacing rule on both layers,
# so the shortest legal window is max over the layers of (enclosure extent along the wire + spacing).
# MEASURED from the sky130 tech (horner odb): M1M2 met1 320+140 -> 460; M2M3 met3 330+300 -> 630;
# M3M4 met3 380+300 -> 680. The fixed 500/340/240/180 list made every met3->met4 window self-notch,
# so net580 on fabric_horner_seq found none at all.
check("met1->met2 shortest legal window", jog.min_bridge_window({"met1": 320, "met2": 260}, {"met1": 140, "met2": 140}), 460)
check("met2->met3 shortest legal window", jog.min_bridge_window({"met2": 280, "met3": 330}, {"met2": 140, "met3": 300}), 630)
check("met3->met4 shortest legal window", jog.min_bridge_window({"met3": 380, "met4": 330}, {"met3": 300, "met4": 300}), 680)
check("widths never go below it, longest first", jog.bridge_widths(680), [680])
check("met1->met2 keeps 500 and adds its own minimum", jog.bridge_widths(460), [500, 460])

# MINIMUM AREA. MEASURED: fabric_horner_seq net580 (RUN_2026.09.24_00.25.18) — the met3->met4 bridge
# left, between its via at p1=(657230,684420) and the net's own M2M3 via at B=(657570,684420), a met3
# island of only 0.229 um^2 against sky130's 0.24 minimum (golden sky130A_mr m3.6; Magic silent).
stub = (657230, 684270, 657570, 684570)       # met3 stub p1..B at its 300 nm width
enc_p1 = (657040, 684260, 657420, 684580)     # the bridge's M3M4 met3 enclosure (380 x 320)
b_via = (657405, 684255, 657735, 684585)      # the net's own M2M3 met3 enclosure at B (330 x 330)
far = (900000, 900000, 901000, 901000)        # own metal elsewhere, not touching
area = jog.connected_area([stub, enc_p1], [b_via, far])
check("the leftover met3 island is under minimum area", area < 240000, True)
check("union area counts the overlap once", jog.union_area([(0, 0, 100, 100), (50, 0, 150, 100)]), 15000)
check("a stub that reaches a long wire is not an island",
      jog.connected_area([stub, enc_p1], [b_via, (657500, 684270, 700000, 684570)]) >= 240000, True)

# BRIDGE-LAYER CLEARANCE BY THE REAL SHAPES. MEASURED: coproc_core R[1][20] (RUN_2026.09.24_00.26.17) —
# an M2M3 via at (785450, 638690) put its 330 nm met3 enclosure (bottom y=638525) 195 nm from foreign
# net _03318_'s 300 nm met3 wire (top y=638330); met3 spacing is 300 (golden m3.2). The window test
# used a +/-100 nm rectangle round the centreline, narrower than the enclosure's +/-165.
enc = (785285, 638525, 785615, 638855)
foreign_wire = (775790, 638030, 810980, 638330)                    # its true 300 nm width
grown = (foreign_wire[0] - 300, foreign_wire[1] - 300, foreign_wire[2] + 300, foreign_wire[3] + 300)
check("the real enclosure hits the foreign wire grown by the spacing rule",
      jog.overlaps(enc, grown), True)

print("PASS: notch guard correct" if bad == 0 else f"FAIL: {bad} case(s)")
sys.exit(1 if bad else 0)

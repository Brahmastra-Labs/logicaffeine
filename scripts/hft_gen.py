#!/usr/bin/env python3
"""hft_gen — emit the LOGOS tick-to-trade pipeline (`.lg`) for a given ITCH-lite byte feed.

Single source of truth for the flagship program: the demo file, the determinism lock, the fuzz oracle,
and the latency harness all get their `.lg` from here, so the on-silicon program and the Python reference
(`hft_model.py`) can never drift — they share the same parameters and the same feed.

The pipeline logic is fixed (decode -> price-ladder book -> microprice market-making -> pre-trade risk ->
order); only the baked feed literal varies. The feed literal is emitted AFTER the book `Push` loop because
the full-core RTL's `NewList` does not advance the heap pointer, so a literal declared before a later
`Push` would be clobbered on silicon.

    python3 scripts/hft_gen.py            # regenerate benchmarks/silicon/programs/hft_pipeline.lg (DEMO_FEED)
    python3 scripts/hft_gen.py --feed 65,1,0,100,...   # emit a program for an explicit feed to stdout
"""
import sys

sys.path.insert(0, sys.path[0] or ".")
import hft_model as M


def gen(feed):
    """Return the full `.lg` source for the pipeline over `feed` (a list of ints, len multiple of 8)."""
    feed_literal = ", ".join(str(b) for b in feed)
    return f"""## Main
Let levels be {M.LEVELS}.
Let fair100 be {M.FAIR_100}.
Let edge100 be {M.EDGE_100}.
Let base_size be {M.BASE_SIZE}.
Let max_order be {M.MAX_ORDER}.
Let pos_cap be {M.POS_CAP}.
Let mutable bid_book be a new Seq of Int.
Let mutable ask_book be a new Seq of Int.
Let mutable z be 0.
While z is less than levels:
    Push 0 to bid_book.
    Push 0 to ask_book.
    Set z to z + 1.
Let feed be [{feed_literal}].
Let n be length of feed.
Let mutable inventory be 0.
Let mutable i be 1.
While i is at most n:
    Let mtype be item i of feed.
    Let side be item (i + 1) of feed.
    Let price be item (i + 2) of feed * 256 + item (i + 3) of feed.
    Let qty be item (i + 4) of feed * 256 + item (i + 5) of feed.
    Let mutable tick be price - {M.PRICE_MIN - 1}.
    If tick is less than 1:
        Set tick to 1.
    If tick is greater than levels:
        Set tick to levels.
    If mtype is equal to {M.TYPE_ADD}:
        If side is equal to 1:
            Set item tick of bid_book to (item tick of bid_book) + qty.
        Otherwise:
            Set item tick of ask_book to (item tick of ask_book) + qty.
    Otherwise:
        If side is equal to 1:
            Set item tick of bid_book to (item tick of bid_book) - qty.
            If (item tick of bid_book) is less than 0:
                Set item tick of bid_book to 0.
        Otherwise:
            Set item tick of ask_book to (item tick of ask_book) - qty.
            If (item tick of ask_book) is less than 0:
                Set item tick of ask_book to 0.
    Let mutable best_bid_tick be 0.
    Let mutable best_ask_tick be levels + 1.
    Let mutable t be 1.
    While t is at most levels:
        If (item t of bid_book) is greater than 0:
            Set best_bid_tick to t.
        If (item t of ask_book) is greater than 0:
            If t is less than best_ask_tick:
                Set best_ask_tick to t.
        Set t to t + 1.
    Let mutable has_bid be 0.
    If best_bid_tick is at least 1:
        Set has_bid to 1.
    Let mutable has_ask be 0.
    If best_ask_tick is at most levels:
        Set has_ask to 1.
    Let book_ok be has_bid * has_ask.
    Let mutable best_bid_price be 0.
    Let mutable best_bid_qty be 0.
    If has_bid is equal to 1:
        Set best_bid_price to best_bid_tick + {M.PRICE_MIN - 1}.
        Set best_bid_qty to item best_bid_tick of bid_book.
    Let mutable best_ask_price be 0.
    Let mutable best_ask_qty be 0.
    If has_ask is equal to 1:
        Set best_ask_price to best_ask_tick + {M.PRICE_MIN - 1}.
        Set best_ask_qty to item best_ask_tick of ask_book.
    Let mutable denom be best_bid_qty + best_ask_qty.
    If denom is less than 1:
        Set denom to 1.
    Let micro100 be (best_bid_price * best_ask_qty + best_ask_price * best_bid_qty) * 100 / denom.
    Let mutable order_side be 0.
    Let mutable order_price be 0.
    Let mutable order_size be 0.
    If book_ok is equal to 1:
        If micro100 is greater than fair100 + edge100:
            Set order_side to 2.
            Set order_price to best_bid_price.
            Set order_size to base_size.
        Otherwise:
            If micro100 is less than fair100 - edge100:
                Set order_side to 1.
                Set order_price to best_ask_price.
                Set order_size to base_size.
    If order_size is greater than max_order:
        Set order_size to max_order.
    Let mutable signed_order be 0.
    If order_side is equal to 1:
        Set signed_order to order_size.
    If order_side is equal to 2:
        Set signed_order to 0 - order_size.
    Let projected be inventory + signed_order.
    Let mutable risk_ok be 1.
    If projected is greater than pos_cap:
        Set risk_ok to 0.
    If projected is less than 0 - pos_cap:
        Set risk_ok to 0.
    If risk_ok is equal to 0:
        Set order_side to 0.
        Set order_price to 0.
        Set order_size to 0.
        Set signed_order to 0.
    Set inventory to inventory + signed_order.
    Show order_side.
    Show order_price.
    Show order_size.
    Set i to i + 8.
"""


def main(argv):
    if len(argv) >= 2 and argv[1] == "--feed":
        feed = [int(x) for x in argv[2].split(",") if x != ""]
        sys.stdout.write(gen(feed))
        return 0
    # default: regenerate the committed demo program
    import os
    out = os.path.join(os.path.dirname(__file__), "..", "benchmarks", "silicon", "programs", "hft_pipeline.lg")
    with open(out, "w") as f:
        f.write(gen(M.DEMO_FEED))
    print(f"wrote {os.path.normpath(out)} ({len(M.DEMO_FEED) // 8} messages, {len(M.DEMO_FEED)} feed bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

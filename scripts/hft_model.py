#!/usr/bin/env python3
"""hft_model — the reference semantics for the LOGOS tick-to-trade demo, in plain integer Python.

This is BOTH the correctness oracle for the LOGOS program (the `.lg` pipeline must produce
byte-identical Show output to `run_pipeline` here) AND the x86 baseline the latency harness times
against the deterministic silicon. It is deliberately written with ordinary branches so it reads as
the spec; the LOGOS port is branchless (constant-time) but must match this output exactly.

Everything is integer arithmetic with truncating division, mirroring the LOGOS 32-bit integer core
(so microprice etc. match the silicon to the unit).

Wire format (ITCH-lite — a compacted, big-endian subset of NASDAQ TotalView-ITCH 5.0; decode cost
scales linearly with field width, so a full-width parser is a projection of this one). Fixed 8-byte
records so a decode loop can index them:

    [type, side, price_hi, price_lo, qty_hi, qty_lo, id_hi, id_lo]

    type : 'A'=65 add resting order, 'D'=68 delete, 'E'=69 execute (delete + it traded)
    side : 1 = buy (bid), 2 = sell (ask)
    price: big-endian u16 absolute price
    qty  : big-endian u16 shares
    id   : big-endian u16 order id (carried on D/E too, so the price-ladder book needs no id->order map;
           a full book keeps that map — the demo carries price in the message to stay O(1) on-chip)

The book is a direct-mapped PRICE LADDER: bid_book[tick] / ask_book[tick] hold resting shares, where
tick = price - PRICE_MIN + 1 (1-based). O(1) update, O(levels) best-of-book scan — exactly how fast
books are built. Prices outside [PRICE_MIN, PRICE_MIN+LEVELS-1] are clamped into the ladder.

The strategy is imbalance/microprice market-making: on every message, recompute top-of-book, compute
the size-weighted microprice, and quote against a fair value — buy when the book is cheap, sell when
rich, gated by pre-trade risk (fat-finger max order + position limit). Every message emits exactly one
order record (size 0 = no trade) so the output shape is constant.
"""

# --- ladder / strategy parameters (shared verbatim with the LOGOS program) ---
PRICE_MIN = 93          # ladder covers PRICE_MIN .. PRICE_MIN+LEVELS-1
LEVELS = 16             # 93..108
# The microprice is size-weighted and always lies in [best_bid, best_ask], so the fair reference sits
# INSIDE the spread (100.50, held in hundredths as 10050) and order-book imbalance moves the microprice
# across it: a heavier bid pulls it up (book rich -> sell), a heavier ask pulls it down (cheap -> buy).
FAIR_100 = 10050        # fair value in hundredths (100.50)
EDGE_100 = 5            # dead-band in hundredths (0.05) around fair -> flat
BASE_SIZE = 10          # target order size
MAX_ORDER = 50          # fat-finger cap
POS_CAP = 100           # |inventory| limit

TYPE_ADD, TYPE_DEL, TYPE_EXE = 65, 68, 69


def encode(msgs):
    """Encode a list of (type, side, price, qty, id) tuples into the ITCH-lite byte feed."""
    out = []
    for (t, side, price, qty, oid) in msgs:
        out += [t & 0xFF, side & 0xFF,
                (price >> 8) & 0xFF, price & 0xFF,
                (qty >> 8) & 0xFF, qty & 0xFF,
                (oid >> 8) & 0xFF, oid & 0xFF]
    return out


def _tick(price):
    t = price - PRICE_MIN + 1
    if t < 1:
        t = 1
    if t > LEVELS:
        t = LEVELS
    return t


def run_pipeline(feed):
    """Run the whole tick-to-trade pipeline over the byte `feed`; return the flat list of Show ints
    (three per message: order_side, order_price, order_size). `feed` length must be a multiple of 8."""
    bid = [0] * (LEVELS + 1)   # 1-based
    ask = [0] * (LEVELS + 1)
    inventory = 0
    shows = []

    n = len(feed)
    i = 0
    while i < n:
        # --- decode one ITCH-lite record ---
        mtype = feed[i]
        side = feed[i + 1]
        price = feed[i + 2] * 256 + feed[i + 3]
        qty = feed[i + 4] * 256 + feed[i + 5]
        tick = _tick(price)

        # --- apply to the book: Add adds resting qty, Delete/Execute remove it ---
        signed = qty if mtype == TYPE_ADD else -qty
        if side == 1:
            bid[tick] = max(0, bid[tick] + signed)
        else:
            ask[tick] = max(0, ask[tick] + signed)

        # --- top of book (best bid = highest populated tick; best ask = lowest) ---
        best_bid_tick = 0
        best_ask_tick = LEVELS + 1
        for t in range(1, LEVELS + 1):
            if bid[t] > 0 and t > best_bid_tick:
                best_bid_tick = t
            if ask[t] > 0 and t < best_ask_tick:
                best_ask_tick = t
        has_bid = 1 if best_bid_tick >= 1 else 0
        has_ask = 1 if best_ask_tick <= LEVELS else 0
        book_ok = has_bid * has_ask

        best_bid_price = best_bid_tick + PRICE_MIN - 1
        best_ask_price = best_ask_tick + PRICE_MIN - 1
        best_bid_qty = bid[best_bid_tick] if has_bid else 0
        best_ask_qty = ask[best_ask_tick] if has_ask else 0

        # --- microprice in hundredths (size-weighted, opposite-side weights) ---
        denom = best_bid_qty + best_ask_qty
        if denom < 1:
            denom = 1
        micro100 = (best_bid_price * best_ask_qty + best_ask_price * best_bid_qty) * 100 // denom

        # --- signal: buy when cheap vs fair, sell when rich (outside the dead-band) ---
        order_side = 0
        order_price = 0
        order_size = 0
        if book_ok:
            if micro100 < FAIR_100 - EDGE_100:
                order_side = 1
                order_price = best_ask_price      # cross the spread to buy
                order_size = BASE_SIZE
            elif micro100 > FAIR_100 + EDGE_100:
                order_side = 2
                order_price = best_bid_price       # cross to sell
                order_size = BASE_SIZE

        # --- pre-trade risk: fat-finger cap + position limit (SEC 15c3-5 in spirit) ---
        if order_size > MAX_ORDER:
            order_size = MAX_ORDER
        signed_order = order_size if order_side == 1 else -order_size
        projected = inventory + signed_order
        if projected > POS_CAP or projected < -POS_CAP:
            order_side = 0
            order_price = 0
            order_size = 0
            signed_order = 0
        inventory += signed_order

        shows += [order_side, order_price, order_size]
        i += 8

    return shows


# A canonical demo feed: build a two-sided book, then walk the microprice across fair so the strategy
# fires buys and sells, then thin the book with executes/deletes.
DEMO_MSGS = [
    (TYPE_ADD, 1, 100, 10, 1),   # bid 100 x10       (one side only -> flat)
    (TYPE_ADD, 2, 101, 10, 2),   # ask 101 x10       (micro 100.50 == fair -> flat)
    (TYPE_ADD, 1, 100, 30, 3),   # bid heavier       (micro rich -> SELL)
    (TYPE_ADD, 2, 101, 60, 4),   # ask heavier       (micro cheap -> BUY)
    (TYPE_ADD, 1, 100, 100, 5),  # bid much heavier  (micro rich -> SELL)
    (TYPE_EXE, 2, 101, 50, 4),   # execute half the ask
    (TYPE_DEL, 1, 100, 100, 5),  # cancel the heavy bid
    (TYPE_ADD, 2, 101, 200, 7),  # ask flood         (micro cheap -> BUY)
]

DEMO_FEED = encode(DEMO_MSGS)


if __name__ == "__main__":
    shows = run_pipeline(DEMO_FEED)
    # print one order per line as "side price size" for eyeballing
    for k in range(0, len(shows), 3):
        print(shows[k], shows[k + 1], shows[k + 2])

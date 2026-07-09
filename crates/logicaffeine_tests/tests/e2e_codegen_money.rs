//! Money across every execution tier — tree-walker, bytecode VM, and AOT-compiled-to-Rust must agree
//! byte-for-byte. Money rides the exact Decimal tower (never float-drifts) and is currency-tagged:
//! same-currency arithmetic only, scaling by a number, exact display at the currency's minor unit.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_compiled_equals_interpreted, assert_interpreter_output};

#[cfg(not(target_arch = "wasm32"))]
const MONEY: &str = "## Main\n\
Let a be money(decimal(\"19.99\"), \"USD\").\n\
Let b be money(decimal(\"5.00\"), \"USD\").\n\
Show a + b.\n\
Show a - b.\n\
Show a * 3.\n\
Show money(10, \"USD\") / 4.\n\
Show money(100, \"JPY\").\n\
Show money(decimal(\"0.10\"), \"USD\") + money(decimal(\"0.20\"), \"USD\").";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_on_interpreter_vm_and_treewalker() {
    // Exact, currency-safe: 24.99, 14.99, 59.97, 2.50 (split), JPY no-decimals, 0.30 (no float drift).
    assert_interpreter_output(
        MONEY,
        "24.99 USD\n14.99 USD\n59.97 USD\n2.50 USD\n100 JPY\n0.30 USD",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_on_aot() {
    common::assert_output_lines(
        MONEY,
        &["24.99 USD", "14.99 USD", "59.97 USD", "2.50 USD", "100 JPY", "0.30 USD"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_all_tiers_agree() {
    assert_compiled_equals_interpreted(MONEY);
}

// ---- NATURAL `19.99 USD` literal syntax (the `2 meters` precedent), forced across every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const NATURAL_MONEY: &str = "## Main\n\
Show 19.99 USD + 5.00 USD.\n\
Show 19.99 USD * 3.\n\
Show 100 JPY.\n\
Let price be 9.99 USD.\nShow price * 2.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_money_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(NATURAL_MONEY, "24.99 USD\n59.97 USD\n100 JPY\n19.98 USD");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_money_on_aot() {
    common::assert_output_lines(NATURAL_MONEY, &["24.99 USD", "59.97 USD", "100 JPY", "19.98 USD"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_money_all_tiers_agree() {
    assert_compiled_equals_interpreted(NATURAL_MONEY);
}

// ---- CURRENCY-MATCH SAFETY: adding incompatible currencies is a COMPILE error (caught by the
//      analysis pass), and rejected pre-execution on the interpreter — like `meter + gram`. ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn currency_mismatch_is_a_compile_error() {
    common::assert_compile_fails("## Main\nShow 5.00 USD + 1.00 EUR.", "LOGOS compile error");
    common::assert_compile_fails("## Main\nShow 5.00 USD + 1.00 EUR.", "different currencies");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn currency_mismatch_through_a_let_is_a_compile_error() {
    common::assert_compile_fails(
        "## Main\nLet price be 9.99 USD.\nLet fee be 2.00 EUR.\nShow price + fee.",
        "different currencies",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn currency_mismatch_rejected_before_execution_on_interpreter() {
    let r = common::run_interpreter("## Main\nShow \"starting\".\nShow 5.00 USD + 1.00 EUR.");
    assert!(!r.success, "interpreter must reject a currency mismatch");
    assert!(r.error.contains("different currencies"), "got: {}", r.error);
    assert!(!r.output.contains("starting"), "should reject before any output; got: {:?}", r.output);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn same_currency_still_compiles_and_runs() {
    common::assert_output_lines("## Main\nShow 5.00 USD + 1.00 USD.", &["6.00 USD"]);
}

// ---- Same-currency comparison + ratio, forced across every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const MONEY_CMP: &str = "## Main\n\
Let a be money(decimal(\"30.00\"), \"USD\").\nLet b be money(decimal(\"10.00\"), \"USD\").\n\
Show a is greater than b.\nShow a / b.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_comparison_and_ratio_on_interpreter_vm_and_treewalker() {
    // 30 > 10 → true; 30/10 → exact ratio 3.
    assert_interpreter_output(MONEY_CMP, "true\n3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_comparison_and_ratio_on_aot() {
    common::assert_output_lines(MONEY_CMP, &["true", "3"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_comparison_and_ratio_all_tiers_agree() {
    assert_compiled_equals_interpreted(MONEY_CMP);
}

// ---- SYMBOL literals: `$19.99`, `€5`, `£10`, `¥100` — the common currency symbols read as money,
//      mapping to their ISO code (USD/EUR/GBP/JPY). Same value as the `19.99 USD` spelled form. ----

#[cfg(not(target_arch = "wasm32"))]
const SYMBOL_MONEY: &str = "## Main\n\
Show $19.99 + $5.00.\n\
Show €5.\n\
Show £10 * 2.\n\
Show ¥100.\n\
Show $1,250.50.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn symbol_money_on_interpreter_vm_and_treewalker() {
    // $ → USD, € → EUR, £ → GBP, ¥ → JPY (no minor unit); thousands separators allowed.
    assert_interpreter_output(SYMBOL_MONEY, "24.99 USD\n5.00 EUR\n20.00 GBP\n100 JPY\n1250.50 USD");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn symbol_money_on_aot() {
    common::assert_output_lines(
        SYMBOL_MONEY,
        &["24.99 USD", "5.00 EUR", "20.00 GBP", "100 JPY", "1250.50 USD"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn symbol_money_all_tiers_agree() {
    assert_compiled_equals_interpreted(SYMBOL_MONEY);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn symbol_and_spelled_money_are_the_same_value() {
    // `$19.99` and `19.99 USD` denote the identical money value.
    common::assert_output_lines("## Main\nShow ($19.99) is equal to (19.99 USD).", &["true"]);
}

// ---- UNIVERSAL MONEY AMOUNT (money's "UTC"): a pluggable ambient rate context lets `<money> in
//      <currency>` convert exactly via the Rational tower. Rates are installed by the program
//      (`Call set_rate with ...`); a program with none in scope errors rather than guessing. Forced
//      across every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const MONEY_CONVERT: &str = "## Main\n\
Call set_rate with \"USD\" and 1.\n\
Call set_rate with \"EUR\" and decimal(\"1.10\").\n\
Call set_rate with \"GBP\" and decimal(\"1.25\").\n\
Show 10.00 EUR in USD.\n\
Show 11.00 USD in EUR.\n\
Show 10.00 GBP in EUR.\n\
Show 42.00 USD in USD.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_conversion_on_interpreter_vm_and_treewalker() {
    // 1 EUR = 1.10 USD, 1 GBP = 1.25 USD, reference USD. Exact, lossless round-trip.
    // 10 EUR → 11.00 USD; 11 USD → 10.00 EUR; 10 GBP = 12.50 USD = 11.36 EUR; identity 42.00 USD.
    assert_interpreter_output(
        MONEY_CONVERT,
        "11.00 USD\n10.00 EUR\n11.36 EUR\n42.00 USD",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_conversion_on_aot() {
    common::assert_output_lines(
        MONEY_CONVERT,
        &["11.00 USD", "10.00 EUR", "11.36 EUR", "42.00 USD"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn money_conversion_all_tiers_agree() {
    assert_compiled_equals_interpreted(MONEY_CONVERT);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn conversion_without_rates_in_scope_is_an_error_on_every_tier() {
    // No rate context installed — conversion must refuse rather than invent a number. The "starting"
    // line proves the program ran up to the bad op, then errored (no silent wrong answer).
    let src = "## Main\nShow \"starting\".\nShow 5.00 USD in EUR.";
    let r = common::run_interpreter(src);
    assert!(!r.success, "must error with no rates in scope");
    assert!(
        r.error.contains("no exchange rates in scope"),
        "got: {}",
        r.error
    );
}

// ---- BULK rate install: load a whole rate table at once from a `Map of Text to <number>` (the
//      "Given rates …" / synced-table source). The same bridge a network-synced or fetched table
//      feeds. Order-independent (a map has unique keys), forced across every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const MONEY_BULK_RATES: &str = "## Main\n\
Let mut rates be a new Map of Text to Decimal.\n\
Set item \"USD\" of rates to decimal(\"1\").\n\
Set item \"EUR\" of rates to decimal(\"1.10\").\n\
Set item \"GBP\" of rates to decimal(\"1.25\").\n\
Call set_rates with rates.\n\
Show 10.00 EUR in USD.\n\
Show 10.00 GBP in EUR.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bulk_rate_install_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(MONEY_BULK_RATES, "11.00 USD\n11.36 EUR");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bulk_rate_install_on_aot() {
    common::assert_output_lines(MONEY_BULK_RATES, &["11.00 USD", "11.36 EUR"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn bulk_rate_install_all_tiers_agree() {
    assert_compiled_equals_interpreted(MONEY_BULK_RATES);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn converting_to_a_currency_with_no_rate_is_an_error() {
    // Rates exist, but not for the target currency — still refuse, never guess.
    let src = "## Main\nCall set_rate with \"USD\" and 1.\nShow 5.00 USD in EUR.";
    let r = common::run_interpreter(src);
    assert!(!r.success, "must error when the target currency has no rate");
    assert!(r.error.contains("no exchange rate"), "got: {}", r.error);
}

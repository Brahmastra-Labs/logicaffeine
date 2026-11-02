use super::*;

/// The ABI counts live handles in one process-wide counter, so a test that compares the count
/// before and after its own work must not run while another test here creates or frees handles.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn shown(h: *const LogosValue) -> String {
    let mut len = 0usize;
    let ptr = unsafe { logos_rt_value_show(h, &mut len) };
    assert!(!ptr.is_null(), "show refused: {}", error());
    let text = unsafe { std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).unwrap().to_string() };
    unsafe { logos_rt_value_text_free(ptr, len) };
    text
}

fn error() -> String {
    let mut len = 0usize;
    let ptr = unsafe { logos_rt_value_error(&mut len) };
    unsafe { std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).unwrap().to_string() }
}

fn text(t: &str) -> *mut LogosValue {
    unsafe { logos_rt_value_text(t.as_ptr(), t.len()) }
}

fn exact(digits: &str) -> *mut LogosValue {
    unsafe { logos_rt_value_exact(digits.as_ptr(), digits.len()) }
}

fn call(name: &str, args: &[*mut LogosValue]) -> *mut LogosValue {
    let args: Vec<*const LogosValue> = args.iter().map(|h| *h as *const LogosValue).collect();
    unsafe { logos_rt_value_call(name.as_ptr(), name.len(), args.as_ptr(), args.len()) }
}

fn dt() -> *mut LogosValue {
    let h = call("infinitesimal", &[text("dt"), logos_rt_value_int(1), logos_rt_value_int(0)]);
    assert!(!h.is_null(), "{}", error());
    h
}

fn drop_all(hs: &[*mut LogosValue]) {
    for h in hs {
        unsafe { logos_rt_value_drop(*h) };
    }
}

#[test]
fn each_standard_value_prints_as_the_interpreter_prints_it() {
    let _serial = serial();
    let cases: Vec<(*mut LogosValue, &str)> = vec![
        (logos_rt_value_int(-42), "-42"),
        (logos_rt_value_float(2.5), "2.5"),
        (logos_rt_value_float(3.0), "3"),
        (logos_rt_value_bool(1), "true"),
        (logos_rt_value_bool(0), "false"),
        (logos_rt_value_nothing(), "nothing"),
        (text("π ≈ 3"), "π ≈ 3"),
        (text(""), ""),
    ];
    for (h, want) in &cases {
        assert_eq!(shown(*h), *want);
    }
    drop_all(&cases.iter().map(|(h, _)| *h).collect::<Vec<_>>());
}

#[test]
fn an_exact_number_is_kept_in_its_smallest_form() {
    let _serial = serial();
    for (digits, want) in [
        ("7/2", "7/2"),
        ("10/5", "2"),
        ("-6/4", "-3/2"),
        ("123456789012345678901234567890", "123456789012345678901234567890"),
        ("0", "0"),
    ] {
        let h = exact(digits);
        assert!(!h.is_null(), "{digits}: {}", error());
        assert_eq!(shown(h), want, "{digits}");
        unsafe { logos_rt_value_drop(h) };
    }
    let two = exact("10/5");
    let mut n = 0i64;
    assert_eq!(unsafe { logos_rt_value_int_of(two, &mut n) }, 1);
    assert_eq!(n, 2, "a whole ratio is an Int");
    unsafe { logos_rt_value_drop(two) };
}

#[test]
fn a_malformed_number_is_refused_not_guessed() {
    let _serial = serial();
    for digits in ["", "1/0", "x", "1/2/3", "1.5"] {
        let h = exact(digits);
        assert!(h.is_null(), "`{digits}` made a value");
        assert_eq!(error(), format!("`{digits}` is not an exact number"));
    }
    let d = unsafe { logos_rt_value_decimal("12.50".as_ptr(), 5) };
    assert!(!d.is_null(), "{}", error());
    assert_eq!(shown(d), "12.50");
    unsafe { logos_rt_value_drop(d) };
    let bad = unsafe { logos_rt_value_decimal("1/2".as_ptr(), 3) };
    assert!(bad.is_null());
    assert_eq!(error(), "`1/2` is not a decimal");
}

#[test]
fn arithmetic_answers_as_the_value_model_does() {
    let _serial = serial();
    let (a, b) = (logos_rt_value_int(7), logos_rt_value_int(2));
    let ops: [(unsafe extern "C" fn(*const LogosValue, *const LogosValue) -> *mut LogosValue, fn(&LogosValue, &LogosValue) -> Result<LogosValue, String>); 8] = [
        (logos_rt_value_add, LogosValue::add),
        (logos_rt_value_sub, LogosValue::sub),
        (logos_rt_value_mul, LogosValue::mul),
        (logos_rt_value_div, LogosValue::div),
        (logos_rt_value_exact_div, LogosValue::exact_div),
        (logos_rt_value_floor_div, LogosValue::floor_div),
        (logos_rt_value_rem, LogosValue::rem),
        (logos_rt_value_pow, LogosValue::pow),
    ];
    for (abi, model) in ops {
        let h = unsafe { abi(a, b) };
        let want = model(&LogosValue::Int(7), &LogosValue::Int(2)).unwrap();
        assert_eq!(shown(h), want.display_string());
        unsafe { logos_rt_value_drop(h) };
    }
    let zero = logos_rt_value_int(0);
    let h = unsafe { logos_rt_value_div(a, zero) };
    assert!(h.is_null(), "a division by zero made a value");
    assert_eq!(Some(error()), LogosValue::Int(7).div(&LogosValue::Int(0)).err());
    drop_all(&[a, b, zero]);
}

#[test]
fn every_relation_answers_a_bool() {
    let _serial = serial();
    let (a, b) = (exact("1/3"), exact("1/2"));
    let rels: [(unsafe extern "C" fn(*const LogosValue, *const LogosValue) -> *mut LogosValue, &str); 6] = [
        (logos_rt_value_eq, "false"),
        (logos_rt_value_ne, "true"),
        (logos_rt_value_lt, "true"),
        (logos_rt_value_gt, "false"),
        (logos_rt_value_le, "true"),
        (logos_rt_value_ge, "false"),
    ];
    for (rel, want) in rels {
        let h = unsafe { rel(a, b) };
        assert_eq!(shown(h), want);
        unsafe { logos_rt_value_drop(h) };
    }
    drop_all(&[a, b]);
}

#[test]
fn a_builtin_of_the_extension_is_called_by_its_name() {
    let _serial = serial();
    let two = logos_rt_value_int(2);
    let root = call("exactSqrt", &[two]);
    assert!(!root.is_null(), "{}", error());
    assert_eq!(shown(root), "√2");
    let square = unsafe { logos_rt_value_mul(root, root) };
    assert_eq!(shown(square), "2");
    let pi = call("exactPi", &[]);
    assert_eq!(shown(pi), "π");
    let omega = call("exactOmega", &[]);
    let one = logos_rt_value_int(1);
    let after = unsafe { logos_rt_value_add(omega, one) };
    assert_eq!(shown(after), "ω + 1");
    drop_all(&[two, root, square, pi, omega, one, after]);
}

#[test]
fn a_builtin_refuses_in_the_interpreter_s_words() {
    let _serial = serial();
    let zero = logos_rt_value_int(0);
    let h = call("exactLn", &[zero]);
    assert!(h.is_null());
    let want = logicaffeine_base::extension::call(ExtensionBuiltin::ExactLn, vec![LogosValue::Int(0)]).unwrap_err();
    assert_eq!(error(), want);
    let unknown = call("noSuchBuiltin", &[]);
    assert!(unknown.is_null());
    assert_eq!(error(), "`noSuchBuiltin` is not a builtin of the exact extension");
    unsafe { logos_rt_value_drop(zero) };
}

#[test]
fn a_panic_inside_the_runtime_is_answered_never_unwound() {
    let _serial = serial();
    let h = call("exactSqrt", &[]);
    assert!(h.is_null(), "a builtin called without its argument made a value");
    assert_eq!(error(), "the checker counted this builtin's arguments");
}

#[test]
fn the_kept_sentence_is_the_latest_refusal() {
    let _serial = serial();
    let zero = logos_rt_value_int(0);
    assert!(call("exactLn", &[zero]).is_null());
    let first = error();
    assert!(exact("x").is_null());
    assert_ne!(error(), first);
    assert_eq!(error(), "`x` is not an exact number");
    unsafe { logos_rt_value_drop(zero) };
}

#[test]
fn a_list_is_built_from_handles_it_does_not_take() {
    let _serial = serial();
    let (a, b) = (logos_rt_value_int(1), exact("1/2"));
    let items = [a as *const LogosValue, b as *const LogosValue];
    let xs = unsafe { logos_rt_value_list(items.as_ptr(), 2) };
    assert_eq!(shown(xs), "[1, 1/2]");
    assert_eq!(shown(a), "1", "the item's own handle still holds it");
    let empty = unsafe { logos_rt_value_list(std::ptr::null(), 0) };
    assert_eq!(shown(empty), "[]");
    let pair = unsafe { logos_rt_value_tuple(items.as_ptr(), 2) };
    assert_eq!(shown(pair), "(1, 1/2)");
    drop_all(&[a, b, xs, empty, pair]);
}

#[test]
fn a_list_grows_and_shrinks_in_place() {
    let _serial = serial();
    let xs = unsafe { logos_rt_value_list(std::ptr::null(), 0) };
    let d = dt();
    assert_eq!(unsafe { logos_rt_value_push(xs, d) }, 1, "{}", error());
    let three = logos_rt_value_int(3);
    assert_eq!(unsafe { logos_rt_value_push(xs, three) }, 1);
    assert_eq!(shown(xs), "[dt, 3]");
    let mut n = 0i64;
    assert_eq!(unsafe { logos_rt_value_length(xs, &mut n) }, 1);
    assert_eq!(n, 2);
    let mut found = 0i32;
    assert_eq!(unsafe { logos_rt_value_contains(xs, three, &mut found) }, 1);
    assert_eq!(found, 1);
    let first = unsafe { logos_rt_value_item_at(xs, 1) };
    assert_eq!(shown(first), "dt");
    let one = logos_rt_value_int(1);
    let seven = logos_rt_value_int(7);
    assert_eq!(unsafe { logos_rt_value_set_item(xs, one, seven) }, 1);
    assert_eq!(shown(xs), "[7, 3]");
    let last = unsafe { logos_rt_value_pop(xs) };
    assert_eq!(shown(last), "3");
    assert_eq!(shown(xs), "[7]");
    let past = unsafe { logos_rt_value_item_at(xs, 5) };
    assert!(past.is_null(), "an item past the end made a value");
    assert_eq!(Some(error()), LogosValue::list(vec![LogosValue::Int(7)]).item(&LogosValue::Int(5)).err());
    assert_eq!(unsafe { logos_rt_value_push(three, one) }, 0, "an Int took a push");
    assert_eq!(error(), "Can only push to a List");
    drop_all(&[xs, d, three, first, one, seven, last]);
}

#[test]
fn a_slice_is_the_inclusive_run_of_a_lists_items_counted_from_one() {
    let _serial = serial();
    let items: Vec<*mut LogosValue> = (1..=5).map(|n| logos_rt_value_int(n * 10)).collect();
    let held: Vec<*const LogosValue> = items.iter().map(|h| *h as *const LogosValue).collect();
    let xs = unsafe { logos_rt_value_list(held.as_ptr(), held.len()) };
    let (two, four) = (logos_rt_value_int(2), logos_rt_value_int(4));
    let run = unsafe { logos_rt_value_slice(xs, two, four) };
    assert_eq!(shown(run), "[20, 30, 40]");
    assert_eq!(shown(xs), "[10, 20, 30, 40, 50]", "the list it is taken from keeps every item");
    let six = logos_rt_value_int(6);
    let past = unsafe { logos_rt_value_slice(xs, two, six) };
    assert_eq!(shown(past), "[]", "a run that reaches past the end is empty");
    let backwards = unsafe { logos_rt_value_slice(xs, four, two) };
    assert_eq!(shown(backwards), "[]");
    let half = call("infinitesimal", &[text("dt"), logos_rt_value_int(1), logos_rt_value_int(0)]);
    let refused = unsafe { logos_rt_value_slice(xs, half, four) };
    assert!(refused.is_null(), "a slice from a value that is no integer made a list");
    assert_eq!(error(), "Slice requires List and Int indices");
    let not_a_list = unsafe { logos_rt_value_slice(two, two, four) };
    assert!(not_a_list.is_null());
    assert_eq!(error(), "Slice requires List and Int indices");
    let mut all = items;
    all.extend([xs, two, four, run, six, past, backwards, half]);
    drop_all(&all);
}

#[test]
fn a_clone_is_a_value_of_its_own() {
    let _serial = serial();
    let one = logos_rt_value_int(1);
    let xs = unsafe { logos_rt_value_list([one as *const LogosValue].as_ptr(), 1) };
    let ys = unsafe { logos_rt_value_clone(xs) };
    assert_eq!(unsafe { logos_rt_value_push(xs, one) }, 1);
    assert_eq!(shown(xs), "[1, 1]");
    assert_eq!(shown(ys), "[1]", "growing a list grew its copy");
    drop_all(&[one, xs, ys]);
}

#[test]
fn a_condition_reads_a_decided_truth_and_refuses_an_undecided_one() {
    let _serial = serial();
    let yes = logos_rt_value_bool(1);
    let mut truth = -1i32;
    assert_eq!(unsafe { logos_rt_value_truth(yes, &mut truth) }, 1);
    assert_eq!(truth, 1);
    let d = dt();
    let one = logos_rt_value_int(1);
    let two = logos_rt_value_int(2);
    let n = unsafe { logos_rt_value_div(one, d) };
    let parity = unsafe { logos_rt_value_rem(n, two) };
    let zero = logos_rt_value_int(0);
    let even = unsafe { logos_rt_value_eq(parity, zero) };
    assert!(!even.is_null(), "{}", error());
    assert_eq!(unsafe { logos_rt_value_truth(even, &mut truth) }, 0, "a residue-dependent answer decided a condition");
    let model = unsafe { &*even }.decided_truth().unwrap_err();
    assert_eq!(error(), model);
    drop_all(&[yes, d, one, two, n, parity, zero, even]);
}

#[test]
fn an_int_is_read_back_and_anything_else_is_refused() {
    let _serial = serial();
    let mut n = 0i64;
    let h = logos_rt_value_int(i64::MIN);
    assert_eq!(unsafe { logos_rt_value_int_of(h, &mut n) }, 1);
    assert_eq!(n, i64::MIN);
    let wide = exact("123456789012345678901234567890");
    assert_eq!(unsafe { logos_rt_value_int_of(wide, &mut n) }, 0);
    assert_eq!(error(), crate::failure::INT_TOO_WIDE);
    let d = dt();
    assert_eq!(unsafe { logos_rt_value_int_of(d, &mut n) }, 0);
    assert_eq!(error(), "Expected Int, found Hyperreal");
    drop_all(&[h, wide, d]);
}

#[test]
fn every_unary_operation_answers_as_the_value_model_does() {
    let _serial = serial();
    let v = exact("-7/2");
    let ops: [(unsafe extern "C" fn(*const LogosValue) -> *mut LogosValue, fn(&LogosValue) -> Result<LogosValue, String>); 5] = [
        (logos_rt_value_abs, LogosValue::abs),
        (logos_rt_value_floor, LogosValue::floor),
        (logos_rt_value_ceil, LogosValue::ceil),
        (logos_rt_value_round, LogosValue::round),
        (logos_rt_value_not, LogosValue::not),
    ];
    let model = unsafe { &*v }.clone();
    for (abi, f) in ops {
        let h = unsafe { abi(v) };
        match f(&model) {
            Ok(want) => assert_eq!(shown(h), want.display_string()),
            Err(why) => {
                assert!(h.is_null());
                assert_eq!(error(), why);
            }
        }
        unsafe { logos_rt_value_drop(h) };
    }
    let four = logos_rt_value_int(4);
    let root = unsafe { logos_rt_value_sqrt(four) };
    assert_eq!(shown(root), LogosValue::Int(4).sqrt().unwrap().display_string());
    drop_all(&[v, four, root]);
}

#[test]
fn a_missing_handle_is_refused() {
    let _serial = serial();
    let one = logos_rt_value_int(1);
    let h = unsafe { logos_rt_value_add(one, std::ptr::null()) };
    assert!(h.is_null());
    assert_eq!(error(), "a value handle is missing");
    unsafe { logos_rt_value_drop(std::ptr::null_mut()) };
    unsafe { logos_rt_value_drop(one) };
}

#[test]
fn a_time_value_is_held_as_the_temporal_value_it_is() {
    let _serial = serial();
    let date = logos_rt_value_date(20454);
    assert_eq!(shown(date), crate::temporal::LogosDate(20454).to_string());
    let span = logos_rt_value_span(0, 14);
    let later = unsafe { logos_rt_value_add(date, span) };
    assert!(!later.is_null(), "{}", error());
    assert_eq!(shown(later), (crate::temporal::LogosDate(20454) + crate::temporal::LogosSpan::new(0, 14)).to_string());
    let earlier = unsafe { logos_rt_value_lt(date, later) };
    assert_eq!(shown(earlier), "true");
    let moment = logos_rt_value_moment(86_400_000_000_000);
    assert_eq!(shown(moment), crate::temporal::LogosMoment(86_400_000_000_000).to_string());
    let noon = logos_rt_value_time(43_200_000_000_000);
    assert_eq!(shown(noon), crate::temporal::LogosTime(43_200_000_000_000).to_string());
    drop_all(&[date, span, later, earlier, moment, noon]);
}

#[test]
fn a_repeat_takes_the_items_once() {
    let _serial = serial();
    let (a, b) = (logos_rt_value_int(1), exact("1/2"));
    let xs = unsafe { logos_rt_value_list([a as *const LogosValue, b as *const LogosValue].as_ptr(), 2) };
    let items = unsafe { logos_rt_value_items(xs) };
    assert_eq!(shown(items), "[1, 1/2]");
    let word = text("añb");
    let chars = unsafe { logos_rt_value_items(word) };
    assert_eq!(shown(chars), "[a, ñ, b]");
    let n = logos_rt_value_int(3);
    assert!(unsafe { logos_rt_value_items(n) }.is_null());
    assert_eq!(Some(error()), LogosValue::Int(3).items().err());
    drop_all(&[a, b, xs, items, word, chars, n]);
}

/// A list of the values `items` hold.
fn list(items: &[*mut LogosValue]) -> *mut LogosValue {
    let items: Vec<*const LogosValue> = items.iter().map(|h| *h as *const LogosValue).collect();
    unsafe { logos_rt_value_list(items.as_ptr(), items.len()) }
}

#[test]
fn a_number_held_in_rust_crosses_as_its_value_in_its_smallest_form() {
    let _serial = serial();
    let small = logicaffeine_base::numeric::BigInt::from_i64(42);
    let h = unsafe { logos_rt_value_of_bigint(&small) };
    assert_eq!(shown(h), "42");
    assert!(matches!(unsafe { &*h }, LogosValue::Int(42)), "an integer that fits a word is an Int");
    let wide = logicaffeine_base::numeric::BigInt::from_i64(i64::MAX).mul(&logicaffeine_base::numeric::BigInt::from_i64(4));
    let w = unsafe { logos_rt_value_of_bigint(&wide) };
    assert_eq!(shown(w), "36893488147419103228");
    let third = Rational::parse("1/3").expect("a rational");
    let r = unsafe { logos_rt_value_of_rational(&third) };
    assert_eq!(shown(r), "1/3");
    let whole = Rational::parse("6/3").expect("a rational");
    let q = unsafe { logos_rt_value_of_rational(&whole) };
    assert!(matches!(unsafe { &*q }, LogosValue::Int(2)), "a whole rational is its integer");
    let price = Decimal::parse("12.50").expect("a decimal");
    let d = unsafe { logos_rt_value_of_decimal(&price) };
    assert_eq!(shown(d), "12.50");
    drop_all(&[h, w, r, q, d]);
}

#[test]
fn a_tuple_of_a_list_holds_its_items_in_order() {
    let _serial = serial();
    let xs = list(&[logos_rt_value_int(1), text("a"), dt()]);
    let t = unsafe { logos_rt_value_tuple_of(xs) };
    assert!(!t.is_null(), "{}", error());
    assert_eq!(shown(t), "(1, a, dt)");
    let not_a_list = logos_rt_value_int(3);
    assert!(unsafe { logos_rt_value_tuple_of(not_a_list) }.is_null());
    assert_eq!(error(), "Expected List, found Int");
    drop_all(&[xs, t, not_a_list]);
}

#[test]
fn a_builtin_applied_by_its_place_is_the_builtin_called_by_its_name() {
    let _serial = serial();
    let index = ExtensionBuiltin::ALL
        .iter()
        .position(|b| b.name() == "infinitesimal")
        .expect("the infinitesimal is a builtin of the extension");
    let args = list(&[text("dt"), logos_rt_value_int(1), logos_rt_value_int(0)]);
    let applied = unsafe { logos_rt_value_apply(index, args) };
    assert!(!applied.is_null(), "{}", error());
    let named = dt();
    assert_eq!(shown(applied), shown(named));
    let past = unsafe { logos_rt_value_apply(ExtensionBuiltin::ALL.len(), args) };
    assert!(past.is_null());
    assert_eq!(error(), format!("no builtin of the exact extension at {}", ExtensionBuiltin::ALL.len()));
    let wrong = list(&[logos_rt_value_int(1), logos_rt_value_int(1), logos_rt_value_int(1)]);
    assert!(unsafe { logos_rt_value_apply(index, wrong) }.is_null());
    assert_eq!(error(), "an infinitesimal's name is Text, got Int", "the builtin's own refusal is kept");
    drop_all(&[args, applied, named, wrong]);
}

/// The digits of 1/7 in base 10 as a compiled program answers them: the place's digit as a fresh handle, counting
/// each read in the cell `env` points at.
unsafe extern "C" fn seventh_digit(code: *mut std::ffi::c_void, env: *mut std::ffi::c_void, n: i64) -> *mut LogosValue {
    assert!(code.is_null());
    *(env as *mut u64) += 1;
    let digits = [1, 4, 2, 8, 5, 7];
    logos_rt_value_int(if n == 0 { 0 } else { digits[((n - 1) % 6) as usize] })
}

/// A function whose every answer is a division by zero, refused with the runtime's reason kept.
unsafe extern "C" fn refusing(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void, _: i64) -> *mut LogosValue {
    let (one, zero) = (logos_rt_value_int(1), logos_rt_value_int(0));
    let answer = logos_rt_value_div(one, zero);
    logos_rt_value_drop(one);
    logos_rt_value_drop(zero);
    answer
}

/// An answer that is no number.
unsafe extern "C" fn wordy(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void, _: i64) -> *mut LogosValue {
    text("seven")
}

fn defined(kind: &str, base: i64, name: &str, call: Definer, env: *mut std::ffi::c_void) -> *mut LogosValue {
    unsafe {
        logos_rt_value_defined_real(
            kind.as_ptr(),
            kind.len(),
            base,
            name.as_ptr(),
            name.len(),
            call,
            std::ptr::null_mut(),
            env,
        )
    }
}

#[test]
fn a_real_a_compiled_function_defines_reads_the_function_only_as_a_refinement_asks() {
    let _serial = serial();
    let mut reads = 0u64;
    let env = &mut reads as *mut u64 as *mut std::ffi::c_void;
    let r = defined("digits", 10, "seventh", seventh_digit, env);
    assert!(!r.is_null(), "{}", error());
    assert_eq!(reads, 0, "defining the real reads none of its digits");
    let below = exact("142857/1000000");
    let above = exact("142858/1000000");
    let (gt, lt) = unsafe { (logos_rt_value_gt(r, below), logos_rt_value_lt(r, above)) };
    assert_eq!((shown(gt), shown(lt)), ("true".to_string(), "true".to_string()));
    assert!(reads > 0 && reads < 64, "the comparisons read {reads} digits");
    let before = reads;
    assert_eq!(shown(r), "0.142857142857142857142857142857");
    assert!(reads > before);
    let million = logos_rt_value_int(1_000_000);
    let scaled = unsafe { logos_rt_value_mul(r, million) };
    let floor = unsafe { logos_rt_value_floor(scaled) };
    assert_eq!(shown(floor), "142857");
    drop_all(&[r, below, above, gt, lt, million, scaled, floor]);
}

#[test]
fn a_definition_the_runtime_cannot_read_is_refused_in_its_words() {
    let _serial = serial();
    let none = std::ptr::null_mut();
    assert!(defined("digits", 1, "seventh", seventh_digit, none).is_null());
    assert_eq!(error(), "a real number's digits are in a base from 2 up, got 1");
    assert!(defined("pixels", 10, "seventh", seventh_digit, none).is_null());
    assert_eq!(error(), "a real number is defined by its digits or its approximations, got pixels");

    let r = defined("approximations", 2, "halves", wordy, none);
    assert!(!r.is_null(), "{}", error());
    assert!(unsafe { logos_rt_value_floor(r) }.is_null());
    assert!(error().contains("halves(") && error().contains("is Text"), "{}", error());

    let why = unsafe {
        let (one, zero) = (logos_rt_value_int(1), logos_rt_value_int(0));
        assert!(logos_rt_value_div(one, zero).is_null());
        drop_all(&[one, zero]);
        error()
    };
    let r = defined("digits", 2, "refusing", refusing, none);
    let one = logos_rt_value_int(1);
    assert!(unsafe { logos_rt_value_lt(r, one) }.is_null());
    assert!(error().contains(&why), "{} does not carry {why}", error());
    drop_all(&[r, one]);
}

#[test]
fn every_answer_the_runtime_reads_is_given_back() {
    let _serial = serial();
    let live = logos_rt_value_live();
    let mut reads = 0u64;
    let r = defined("digits", 10, "seventh", seventh_digit, &mut reads as *mut u64 as *mut std::ffi::c_void);
    let s = shown(r);
    unsafe { logos_rt_value_drop(r) };
    assert!(s.starts_with("0.142857"));
    assert_eq!(logos_rt_value_live(), live, "{reads} answers read, and a handle is still held");
}

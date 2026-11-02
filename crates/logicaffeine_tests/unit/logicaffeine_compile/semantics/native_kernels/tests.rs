use super::*;

#[test]
fn codes_names_and_the_table_agree() {
    for (i, &k) in NativeKernel::ALL.iter().enumerate() {
        assert_eq!(k.code(), i as i64);
        assert_eq!(NativeKernel::from_code(k.code()), Some(k));
        assert_eq!(NativeKernel::from_name(k.name()), Some(k));
    }
    assert_eq!(NativeKernel::from_code(-1), None);
    assert_eq!(
        NativeKernel::from_code(NativeKernel::ALL.len() as i64),
        None
    );
    assert_eq!(NativeKernel::from_name("notAKernel"), None);
}

#[test]
fn a_wrong_argument_is_a_named_error_never_a_silent_value() {
    let text = RuntimeValue::Text(Rc::new("x".into()));
    assert_eq!(
        NativeKernel::ZerosW16.call(&[text]).unwrap_err(),
        "zerosW16() needs an Int here, got Text"
    );
    assert_eq!(
        NativeKernel::ZerosW16.call(&[]).unwrap_err(),
        "zerosW16() takes 1 argument, got 0"
    );
    let ints = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(vec![1, 2]))));
    assert_eq!(
        NativeKernel::ToMontW16.call(&[ints]).unwrap_err(),
        "toMontW16() needs a Seq of Word16 here; item 1 is Int"
    );
}

#[test]
fn a_word16_kernel_returns_word16s() {
    let out = NativeKernel::ZerosW16
        .call(&[RuntimeValue::Int(3)])
        .unwrap();
    let RuntimeValue::List(l) = out else {
        panic!("a Seq")
    };
    assert_eq!(
        l.borrow().to_values(),
        vec![RuntimeValue::Word(WordVal::W16(Word16(0))); 3]
    );
}

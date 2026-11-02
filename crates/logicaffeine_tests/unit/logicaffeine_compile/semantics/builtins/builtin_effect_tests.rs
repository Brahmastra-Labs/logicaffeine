use super::{BuiltinEffect, BuiltinId};

#[test]
fn representative_builtins_carry_their_effect() {
    for b in [
        BuiltinId::Length,
        BuiltinId::Format,
        BuiltinId::Abs,
        BuiltinId::CountOnes,
        BuiltinId::Word32,
    ] {
        assert_eq!(b.effect(), BuiltinEffect::Pure, "{b:?}");
    }
    for b in [
        BuiltinId::ParseInt,
        BuiltinId::Chr,
        BuiltinId::TextFromBytes,
        BuiltinId::Fail,
        BuiltinId::FailReason,
    ] {
        assert_eq!(
            b.effect(),
            BuiltinEffect::Fallible,
            "{b:?} can stop the program"
        );
    }
    for b in [BuiltinId::RandomInt, BuiltinId::RandomFloat, BuiltinId::TimeNow] {
        assert_eq!(b.effect(), BuiltinEffect::Nondeterministic, "{b:?}");
    }
    for b in [
        BuiltinId::FileRead,
        BuiltinId::FileWrite,
        BuiltinId::EnvGet,
        BuiltinId::EnvHas,
        BuiltinId::EnvValue,
        BuiltinId::DomSetText,
        BuiltinId::StorageSet,
        BuiltinId::SetRate,
    ] {
        assert_eq!(
            b.effect(),
            BuiltinEffect::Effectful,
            "{b:?} touches the world"
        );
    }
}

#[test]
fn no_browser_builtin_is_pure() {
    for &b in BuiltinId::ALL {
        if b.is_browser_only() {
            assert_eq!(
                b.effect(),
                BuiltinEffect::Effectful,
                "{b:?} is browser-only, so it touches the page"
            );
        }
    }
}

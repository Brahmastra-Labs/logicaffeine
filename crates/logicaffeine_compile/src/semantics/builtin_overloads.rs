//! How the self-interpreter runs each builtin. For a builtin the self-interpreter calls itself, the ways it can be
//! called: each overload's argument types in order and its answer's type, as LOGOS writes them, tried in order, and the
//! report a call stops with when no overload takes its arguments, in the tree-walker's words. The self-interpreter's
//! builtin dispatch is generated from this table (`optimize::core_builtins`), so a builtin it runs is the language's
//! own, called on the values its arguments carry.
//!
//! A builtin that refuses some argument types in words of its own lists them as [`Stop`]s, which the self-interpreter
//! checks before the overloads, so it refuses them in those words.
//!
//! A `Real` parameter takes an `Int` argument too, widened, as the tree-walker widens one (`sqrt(4)`, `min(1, 2.5)`).
//! `Bytes` is a list of `Int`s read as bytes, as every builtin over bytes reads one: an item that is not an `Int` stops
//! the call with [`BYTE_ELEMENT_REFUSAL`]. A refusal names the type of the argument at position `n` as `{n}`.

use super::builtins::BuiltinId;

/// The report a `Bytes` argument stops with when its item `n` is not an `Int` (`semantics::builtins::byte_seq`'s).
pub const BYTE_ELEMENT_REFUSAL: &str = "expected a Seq of Int (bytes); element {n} is not an Int";

/// The report a `Bytes` argument that is no list stops with.
const NOT_BYTES: &str = "expected a Seq of Int (bytes), got {1}";

/// One way of calling a builtin: the types of its arguments, in order, and of its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overload {
    pub params: &'static [&'static str],
    pub answer: &'static str,
}

/// Argument types the language refuses in words of its own for a builtin the self-interpreter runs: a call whose
/// arguments are of these types stops with `report`, before any overload is tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    pub params: &'static [&'static str],
    pub report: &'static str,
}

/// The report every date component extractor refuses an argument that is neither a Moment nor a Date with.
const NOT_A_CALENDAR_VALUE: &str = "a date component extractor requires a Moment or Date, got {1}";

/// A calendar component of an instant or of a date: an `Int` either way.
const COMPONENT: &[Overload] = &[Overload { params: &["Moment"], answer: "Int" }, Overload { params: &["Date"], answer: "Int" }];

/// A clock component of an instant, an `Int`; a Date has none ([`NO_TIME_OF_DAY`]).
const CLOCK_COMPONENT: &[Overload] = &[Overload { params: &["Moment"], answer: "Int" }];

/// What a builtin that reads a time of day says of a Date, which has none.
const NO_TIME_OF_DAY: &[Stop] = &[Stop { params: &["Date"], report: "a Date has no time-of-day — use a timestamp/Moment" }];

/// How the self-interpreter runs one builtin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfInterpreted {
    /// The builtin itself, called on its arguments' values by the first overload that takes them; a call no overload
    /// takes stops with `refusal`.
    Overloads {
        overloads: &'static [Overload],
        refusal: &'static str,
    },
    /// A definition of the self-interpreter's own (`coreDefined…` in `core_interp.logos`): a builtin of any arity, of
    /// any value, or whose answer's type depends on its arguments' values.
    Defined,
    /// Not run by the self-interpreter yet: the runtime value it needs, which its values do not carry.
    Awaits(&'static str),
}

impl BuiltinId {
    /// How the self-interpreter runs this builtin. Exhaustive: a new builtin does not compile until it is classified.
    /// The argument types the language refuses this builtin in words of its own, which the self-interpreter stops on
    /// before trying its overloads.
    pub fn self_interpreted_stops(self) -> &'static [Stop] {
        use BuiltinId as B;
        match self {
            B::HourOf | B::MinuteOf | B::SecondOf | B::TimeOf => NO_TIME_OF_DAY,
            _ => &[],
        }
    }

    pub fn self_interpreted(self) -> SelfInterpreted {
        use BuiltinId as B;
        use SelfInterpreted::*;
        match self {
            B::Length => Defined,
            B::Contains => Overloads {
                overloads: &[Overload { params: &["Text", "Text"], answer: "Bool" }],
                refusal: "contains expects two Text arguments, got {1} and {2}",
            },
            B::Format => Defined,
            B::ParseInt => Overloads {
                overloads: &[Overload { params: &["Text"], answer: "Int" }],
                refusal: "parseInt requires a Text argument",
            },
            B::ParseFloat => Overloads {
                overloads: &[Overload { params: &["Text"], answer: "Real" }],
                refusal: "parseFloat requires a Text argument",
            },
            B::Chr => Overloads {
                overloads: &[Overload { params: &["Int"], answer: "Text" }],
                refusal: "chr() requires an Int argument",
            },
            B::Abs => Overloads {
                overloads: &[Overload { params: &["Int"], answer: "Int" }, Overload { params: &["Real"], answer: "Real" }],
                refusal: "abs() requires a number, got {1}",
            },
            B::Sqrt => Overloads {
                overloads: &[Overload { params: &["Real"], answer: "Real" }],
                refusal: "sqrt() requires a number, got {1}",
            },
            B::Min => Overloads {
                overloads: &[Overload { params: &["Int", "Int"], answer: "Int" }, Overload { params: &["Real", "Real"], answer: "Real" }],
                refusal: "min() requires numbers",
            },
            B::Max => Overloads {
                overloads: &[Overload { params: &["Int", "Int"], answer: "Int" }, Overload { params: &["Real", "Real"], answer: "Real" }],
                refusal: "max() requires numbers",
            },
            B::Floor => Overloads {
                overloads: &[Overload { params: &["Int"], answer: "Int" }, Overload { params: &["Real"], answer: "Int" }],
                refusal: "floor() requires a number, got {1}",
            },
            B::Ceil => Overloads {
                overloads: &[Overload { params: &["Int"], answer: "Int" }, Overload { params: &["Real"], answer: "Int" }],
                refusal: "ceil() requires a number, got {1}",
            },
            B::Round => Overloads {
                overloads: &[Overload { params: &["Int"], answer: "Int" }, Overload { params: &["Real"], answer: "Int" }],
                refusal: "round() requires a number, got {1}",
            },
            B::Pow => Defined,
            B::Decimal => Awaits("a Decimal"),
            B::Complex => Awaits("a Complex"),
            B::Conjugate => Awaits("a Complex"),
            B::RealPart => Awaits("a Complex"),
            B::ImagPart => Awaits("a Complex"),
            B::Modular => Awaits("a Modular"),
            B::Quantity => Awaits("a Quantity"),
            B::Money => Awaits("Money"),
            B::SetRate => Awaits("Money"),
            B::ToCurrency => Awaits("Money"),
            B::SetRates => Awaits("Money"),
            B::Uuid => Awaits("a Uuid"),
            B::UuidNil => Awaits("a Uuid"),
            B::UuidMax => Awaits("a Uuid"),
            B::UuidVersion => Awaits("a Uuid"),
            B::UuidDns => Awaits("a Uuid"),
            B::UuidUrl => Awaits("a Uuid"),
            B::UuidOid => Awaits("a Uuid"),
            B::UuidX500 => Awaits("a Uuid"),
            B::TextBytes => Overloads {
                overloads: &[Overload { params: &["Text"], answer: "Bytes" }],
                refusal: "text_bytes() requires text, got {1}",
            },
            B::TextFromBytes => Overloads { overloads: &[Overload { params: &["Bytes"], answer: "Text" }], refusal: NOT_BYTES },
            B::WireBytes => Awaits("a value's wire bytes"),
            B::CompressBytes => Awaits("the compression codec on the compiled tier"),
            B::DecompressBytes => Awaits("the compression codec on the compiled tier"),
            B::SealBytes => Awaits("bytes"),
            B::OpenBytes => Awaits("bytes"),
            B::FromWireBytes => Awaits("a value's wire bytes"),
            B::SealFrame => Awaits("bytes"),
            B::OpenFrame => Awaits("bytes"),
            B::ReadWireProgram => Awaits("a value's wire bytes"),
            B::WriteWireResidual => Awaits("a value's wire bytes"),
            B::UuidBytes => Awaits("a Uuid"),
            B::UuidFromBytes => Awaits("a Uuid"),
            B::Convert => Awaits("a Quantity"),
            B::ParseTimestamp => Overloads {
                overloads: &[Overload { params: &["Text"], answer: "Moment" }],
                refusal: "parse_timestamp() requires a Text, got {1}",
            },
            B::FormatTimestamp => Overloads {
                overloads: &[Overload { params: &["Moment"], answer: "Text" }],
                refusal: "format_timestamp() requires a Moment, got {1}",
            },
            B::YearOf | B::MonthOf | B::DayOf | B::WeekdayOf | B::WeekOf | B::QuarterOf => {
                Overloads { overloads: COMPONENT, refusal: NOT_A_CALENDAR_VALUE }
            }
            B::HourOf | B::MinuteOf | B::SecondOf => Overloads { overloads: CLOCK_COMPONENT, refusal: NOT_A_CALENDAR_VALUE },
            B::DateOf => Overloads {
                overloads: &[Overload { params: &["Moment"], answer: "Date" }, Overload { params: &["Date"], answer: "Date" }],
                refusal: "date_of() requires a Moment or Date, got {1}",
            },
            B::TimeOf => Overloads {
                overloads: &[Overload { params: &["Moment"], answer: "Time" }],
                refusal: "time_of() requires a Moment, got {1}",
            },
            B::MakeDate => Overloads {
                overloads: &[Overload { params: &["Int", "Int", "Int"], answer: "Date" }],
                refusal: "makeDate() requires three Ints (year, month, day), got {1}, {2}, {3}",
            },
            B::MakeMoment => Overloads {
                overloads: &[Overload { params: &["Int", "Int", "Int", "Int", "Int", "Int"], answer: "Moment" }],
                refusal: "makeMoment() requires six Ints (year, month, day, hour, minute, second)",
            },
            B::LocalInstant => Overloads {
                overloads: &[Overload { params: &["Moment", "Text"], answer: "Moment" }],
                refusal: "local_instant() requires a Moment and a zone name (Text), got {1} and {2}",
            },
            B::SecondsBetween => Overloads {
                overloads: &[Overload { params: &["Moment", "Moment"], answer: "Int" }],
                refusal: "seconds_between() requires two Moments, got {1} and {2}",
            },
            B::MonthsBetween => Overloads {
                overloads: &[Overload { params: &["Moment", "Moment"], answer: "Int" }],
                refusal: "months_between() requires two Moments, got {1} and {2}",
            },
            B::YearsBetween => Overloads {
                overloads: &[Overload { params: &["Moment", "Moment"], answer: "Int" }],
                refusal: "years_between() requires two Moments, got {1} and {2}",
            },
            B::AddSeconds => Overloads {
                overloads: &[Overload { params: &["Moment", "Int"], answer: "Moment" }],
                refusal: "add_seconds() requires a Moment and an Int, got {1} and {2}",
            },
            B::InZone => Overloads {
                overloads: &[Overload { params: &["Moment", "Text"], answer: "Text" }],
                refusal: "in_zone() requires a Moment and a zone name (Text), got {1} and {2}",
            },
            B::Copy => Defined,
            B::CountOnes => Awaits("a fixed-width word"),
            B::LeadingZeros => Awaits("a fixed-width word"),
            B::TrailingZeros => Awaits("a fixed-width word"),
            B::RunAccepted => Awaits("a function value"),
            B::Word32 => Awaits("a fixed-width word"),
            B::Word64 => Awaits("a fixed-width word"),
            B::Rotl => Awaits("a fixed-width word"),
            B::Rotr => Awaits("a fixed-width word"),
            B::Wand => Awaits("a fixed-width word"),
            B::Wor => Awaits("a fixed-width word"),
            B::Wnot => Awaits("a fixed-width word"),
            B::Lanes4Word32Make => Awaits("SIMD lanes"),
            B::Lanes4Of => Awaits("SIMD lanes"),
            B::SeqOfLanes4W32 => Awaits("SIMD lanes"),
            B::Sha1Rnds4 => Awaits("SIMD lanes"),
            B::Sha1Msg1 => Awaits("SIMD lanes"),
            B::Sha1Msg2 => Awaits("SIMD lanes"),
            B::Sha1Nexte => Awaits("SIMD lanes"),
            B::Lanes16Word8Make => Awaits("SIMD lanes"),
            B::SeqOfLanes16W8 => Awaits("SIMD lanes"),
            B::Splat16Word8 => Awaits("SIMD lanes"),
            B::Shuffle16 => Awaits("SIMD lanes"),
            B::ShrBytes16 => Awaits("SIMD lanes"),
            B::InterleaveLo16 => Awaits("SIMD lanes"),
            B::InterleaveHi16 => Awaits("SIMD lanes"),
            B::ByteAdd16 => Awaits("SIMD lanes"),
            B::Maddubs16 => Awaits("SIMD lanes"),
            B::Packus16 => Awaits("SIMD lanes"),
            B::Lanes8Word32 => Awaits("SIMD lanes"),
            B::SeqOfLanes8 => Awaits("SIMD lanes"),
            B::Splat8Word32 => Awaits("SIMD lanes"),
            B::IntOfWord32 => Awaits("a fixed-width word"),
            B::IntOfWord64 => Awaits("a fixed-width word"),
            B::Word64Shl => Awaits("a fixed-width word"),
            B::Word64Shr => Awaits("a fixed-width word"),
            B::Word64And => Awaits("a fixed-width word"),
            B::Word32Shr => Awaits("a fixed-width word"),
            B::Word16Make => Awaits("a fixed-width word"),
            B::IntOfWord16 => Awaits("a fixed-width word"),
            B::Lanes4Word64 => Awaits("SIMD lanes"),
            B::SeqOfLanes4 => Awaits("SIMD lanes"),
            B::Mul32x32To64 => Awaits("a fixed-width word"),
            B::HsumLanes4 => Awaits("SIMD lanes"),
            B::Splat4Word64 => Awaits("SIMD lanes"),
            B::AndNot4 => Awaits("SIMD lanes"),
            B::Lanes16Word16 => Awaits("SIMD lanes"),
            B::SeqOfLanes16 => Awaits("SIMD lanes"),
            B::Splat16Word16 => Awaits("SIMD lanes"),
            B::Mulhi16 => Awaits("SIMD lanes"),
            B::Montmul32 => Awaits("a fixed-width word"),
            B::NttBcastLo => Awaits("SIMD lanes"),
            B::NttBcastHi => Awaits("SIMD lanes"),
            B::NttBlend => Awaits("SIMD lanes"),
            B::MapOf => Defined,
            B::SetOf => Defined,
            B::RepeatSeq => Defined,
            B::ReedSolomonEncode => Awaits("bytes"),
            B::ReedSolomonRecover => Awaits("bytes"),
            B::FecProtect => Awaits("bytes"),
            B::FecRestore => Awaits("bytes"),
            B::DomBody => Awaits("a page"),
            B::DomCreateElement => Awaits("a page"),
            B::DomCreateTextNode => Awaits("a page"),
            B::DomSetText => Awaits("a page"),
            B::DomSetHtml => Awaits("a page"),
            B::DomClearChildren => Awaits("a page"),
            B::DomOnMessage => Awaits("a page"),
            B::DomMoveChildren => Awaits("a page"),
            B::DomBeginPaint => Awaits("a page"),
            B::DomSetAttribute => Awaits("a page"),
            B::DomAppendChild => Awaits("a page"),
            B::DomGetText => Awaits("a page"),
            B::DomGetValue => Awaits("a page"),
            B::DomSetValue => Awaits("a page"),
            B::DomApplyPatches => Awaits("a page"),
            B::DomSetChecked => Awaits("a page"),
            B::DomPreventDefault => Awaits("a page"),
            B::StorageSet => Awaits("a page"),
            B::StorageGet => Awaits("a page"),
            B::StorageGetOr => Awaits("a page"),
            B::StorageRemove => Awaits("a page"),
            B::DomSetStream => Awaits("a page"),
            B::RtcPeerConnection => Awaits("a page"),
            B::RtcDataChannel => Awaits("a page"),
            B::RtcCreateOffer => Awaits("a page"),
            B::RtcCreateAnswer => Awaits("a page"),
            B::RtcSetLocal => Awaits("a page"),
            B::RtcSetRemote => Awaits("a page"),
            B::RtcAddIce => Awaits("a page"),
            B::RtcSend => Awaits("a page"),
            B::DomQuerySelector => Awaits("a page"),
            B::DomAddEventListener => Awaits("a page"),
            B::DomEventValue => Awaits("a page"),
            B::DomEventKey => Awaits("a page"),
            B::TryReadBytes => Awaits("a file"),
            B::DomEventTarget => Awaits("a page"),
            B::DomFetchText => Awaits("a page"),
            B::DomResponseText => Awaits("a page"),
            B::ReadBytes => Awaits("a file"),
            B::SignalNew => Awaits("a page"),
            B::SignalSet => Awaits("a page"),
            B::SignalGetInt => Awaits("a page"),
            B::SignalGetText => Awaits("a page"),
            B::SignalBeginRender => Awaits("a page"),
            B::SignalNewText => Awaits("a page"),
            B::SignalSetText => Awaits("a page"),
            B::SignalEnterComponent => Awaits("a page"),
            B::SignalExitComponent => Awaits("a page"),
            B::SignalNewSeq => Awaits("a page"),
            B::SignalGetSeq => Awaits("a page"),
            B::SignalSetSeq => Awaits("a page"),
            B::SignalNewFloat => Awaits("a page"),
            B::SignalGetFloat => Awaits("a page"),
            B::SignalSetFloat => Awaits("a page"),
            B::FileRead => Awaits("a file"),
            B::FileWrite => Awaits("a file"),
            B::RandomInt => Awaits("a random draw"),
            B::RandomFloat => Awaits("a random draw"),
            B::DomFetchSend => Awaits("a page"),
            B::DomResponseStatus => Awaits("a page"),
            B::DomResponseBody => Awaits("a page"),
            B::DomResponseHeader => Awaits("a page"),
            B::DomResponseError => Awaits("a page"),
            B::DomFetchAbort => Awaits("a page"),
            B::DomFetchSetTimeout => Awaits("a page"),
            B::DomRoutePath => Awaits("a page"),
            B::DomRouteQuery => Awaits("a page"),
            B::DomNavigate => Awaits("a page"),
            B::DomReplaceRoute => Awaits("a page"),
            B::DomSetTitle => Awaits("a page"),
            B::DomClipboardWrite => Awaits("a page"),
            B::DomDownloadFile => Awaits("a page"),
            B::DomNotify => Awaits("a page"),
            B::DomPublish => Awaits("a page"),
            B::DomElementCommand => Awaits("a page"),
            B::DomEventFileName => Awaits("a page"),
            B::DomSetInterval => Awaits("a page"),
            B::DomScrollGap => Awaits("a page"),
            B::DomOnWindowEvent => Awaits("a page"),
            B::DomGetAttribute => Awaits("a page"),
            B::DomGoBack => Awaits("a page"),
            B::DomReload => Awaits("a page"),
            B::DomOnRoute => Awaits("a page"),
            B::Fail => Awaits("a contract's report"),
            B::FailReason => Awaits("a contract's report"),
            B::Word8Make => Awaits("a fixed-width word"),
            B::IntOfWord8 => Awaits("a fixed-width word"),
            B::NativeKernel => Awaits("a native kernel"),
            B::NoteTurn => Awaits("a page"),
            B::DomSetHeadHtml => Awaits("a page"),
            B::InspectUnhandledDynamic => Awaits("a contract's report"),
            B::DomEventFileSize => Awaits("a page"),
            B::DomEventFileType => Awaits("a page"),
            B::DomEventFileBytes => Awaits("a page"),
            B::DomListenFlags => Awaits("a page"),
            B::DomAssignLocation => Awaits("a page"),
            B::DomJournalDetail => Awaits("a page"),
            B::DomApplyWords => Awaits("a page"),
            B::DomWakeAfter => Awaits("a page"),
            B::DomEnterValue => Awaits("a page"),
            B::DomDispatchEntry => Awaits("a page"),
            B::VfsReadFile => Awaits("a file"),
            B::VfsWriteFile => Awaits("a file"),
            B::VfsMount => Awaits("a file"),
            B::EnvGet => Awaits("the environment"),
            B::TimeNow => Overloads { overloads: &[Overload { params: &[], answer: "Int" }], refusal: "" },
            B::EnvHas => Awaits("the environment"),
            B::EnvValue => Awaits("the environment"),
            B::Infinitesimal => Awaits("an exact real"),
            B::StandardPart => Awaits("an exact real"),
            B::IsInfinitesimal => Awaits("an exact real"),
            B::IsLimited => Awaits("an exact real"),
            B::InfinitelyClose => Awaits("an exact real"),
            B::GeneratorOf => Awaits("an exact real"),
            B::LimitOf => Awaits("an exact real"),
            B::Provisional => Awaits("an exact real"),
            B::AtTheLimit => Awaits("an exact real"),
            B::EveryNatural => Awaits("an exact real"),
            B::TripCount => Awaits("an exact real"),
            B::AffinePower => Awaits("an exact real"),
            B::CycleAt => Awaits("an exact real"),
            B::SearchCandidates => Awaits("an exact real"),
            B::StateSize => Awaits("an exact real"),
            B::PolynomialFlow => Awaits("an exact real"),
            B::ExactSqrt => Awaits("an exact real"),
            B::ExactExp => Awaits("an exact real"),
            B::ExactLn => Awaits("an exact real"),
            B::ExactPi => Awaits("an exact real"),
            B::ExactE => Awaits("an exact real"),
            B::ExactOmega => Awaits("an exact real"),
            B::HybridFlow => Awaits("an exact real"),
            B::FirstExit => Awaits("an exact real"),
            B::DefinedReal => Awaits("an exact real"),
            B::ExactPower => Awaits("an exact real"),
            B::OrderedInfinitesimal => Awaits("an exact real"),
            B::SealBox => Awaits("bytes"),
            B::OpenBox => Awaits("bytes"),
            B::SealValueBox => Awaits("a value's wire bytes"),
            B::OpenValueBox => Awaits("a value's wire bytes"),
            B::ReadReals => Awaits("a file"),
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/builtin_overloads/tests.rs"]
mod tests;

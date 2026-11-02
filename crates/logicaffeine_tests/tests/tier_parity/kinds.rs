//! The denominators: every statement arm, expression arm, binary operator, VM instruction and builtin, each
//! computed from the product rather than listed by a person.
//!
//! [`classify_stmt`], [`classify_expr`] and [`classify_binop`] are total matches with no `_` arm, so a new AST
//! variant does not compile until it is classified here, and [`stmt_probe`] and [`expr_probe`] are total over
//! the kinds, so a classified kind does not compile until it has programs. The instruction set comes from
//! `Op::all_variants()` and the builtins from `BuiltinId::ALL`. [`reached`] reads what a program actually
//! contains, so a probe cannot claim a construct it never uses, and the corpus as a whole must reach every
//! member of every denominator.

use std::collections::BTreeSet;

use logicaffeine_language::ast::stmt::{BinaryOpKind, Expr, Stmt};
use logicaffeine_language::ast_depth::{for_each_node, Visited};

/// One kind per `Stmt` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StmtKind {
    Let,
    Set,
    Swap,
    Call,
    If,
    While,
    Repeat,
    Return,
    Break,
    Assert,
    Trust,
    RuntimeAssert,
    Fail,
    Give,
    Show,
    SetField,
    StructDef,
    FunctionDef,
    TestDef,
    Expect,
    ExpectFail,
    ExpectOutput,
    AdvanceTime,
    GivenFile,
    Observe,
    During,
    ForAll,
    Inspect,
    Push,
    Pop,
    Add,
    Remove,
    SetIndex,
    Splice,
    Reversibly,
    Zone,
    Concurrent,
    Parallel,
    ReadFrom,
    WriteFile,
    Spawn,
    SendMessage,
    AwaitMessage,
    StreamMessage,
    MergeCrdt,
    JoinCrdt,
    IncreaseCrdt,
    DecreaseCrdt,
    AppendToSequence,
    ResolveConflict,
    Check,
    Listen,
    Serve,
    ConnectTo,
    LetPeerAgent,
    Sleep,
    Sync,
    Ui,
    Fetch,
    Replay,
    ServeFetches,
    GivenFetch,
    ComponentDef,
    DirectiveDef,
    RouteDef,
    MiddlewareDef,
    RedirectDef,
    Mount,
    LaunchTask,
    LaunchTaskWithHandle,
    CreatePipe,
    SendPipe,
    ReceivePipe,
    TrySendPipe,
    TryReceivePipe,
    StopTask,
    CloseChannel,
    AwaitTask,
    Select,
    Theorem,
    Definition,
    Axiom,
    Theory,
    Escape,
    Require,
    Site,
}

/// ★ No `_` arm: a new `Stmt` variant does not compile until it is classified.
pub fn classify_stmt(s: &Stmt<'_>) -> StmtKind {
    use StmtKind as K;
    match s {
        Stmt::Let { .. } => K::Let,
        Stmt::Set { .. } => K::Set,
        Stmt::Swap { .. } => K::Swap,
        Stmt::Call { .. } => K::Call,
        Stmt::If { .. } => K::If,
        Stmt::While { .. } => K::While,
        Stmt::Repeat { .. } => K::Repeat,
        Stmt::Return { .. } => K::Return,
        Stmt::Break => K::Break,
        Stmt::Assert { .. } => K::Assert,
        Stmt::Trust { .. } => K::Trust,
        Stmt::RuntimeAssert { .. } => K::RuntimeAssert,
        Stmt::Fail { .. } => K::Fail,
        Stmt::Give { .. } => K::Give,
        Stmt::Show { .. } => K::Show,
        Stmt::SetField { .. } => K::SetField,
        Stmt::StructDef { .. } => K::StructDef,
        Stmt::FunctionDef { .. } => K::FunctionDef,
        Stmt::TestDef { .. } => K::TestDef,
        Stmt::Expect { .. } => K::Expect,
        Stmt::ExpectFail { .. } => K::ExpectFail,
        Stmt::ExpectOutput { .. } => K::ExpectOutput,
        Stmt::AdvanceTime { .. } => K::AdvanceTime,
        Stmt::GivenFile { .. } => K::GivenFile,
        Stmt::Observe { .. } => K::Observe,
        Stmt::During { .. } => K::During,
        Stmt::ForAll { .. } => K::ForAll,
        Stmt::Inspect { .. } => K::Inspect,
        Stmt::Push { .. } => K::Push,
        Stmt::Pop { .. } => K::Pop,
        Stmt::Add { .. } => K::Add,
        Stmt::Remove { .. } => K::Remove,
        Stmt::SetIndex { .. } => K::SetIndex,
        Stmt::Splice { .. } => K::Splice,
        Stmt::Reversibly { .. } => K::Reversibly,
        Stmt::Zone { .. } => K::Zone,
        Stmt::Concurrent { .. } => K::Concurrent,
        Stmt::Parallel { .. } => K::Parallel,
        Stmt::ReadFrom { .. } => K::ReadFrom,
        Stmt::WriteFile { .. } => K::WriteFile,
        Stmt::Spawn { .. } => K::Spawn,
        Stmt::SendMessage { .. } => K::SendMessage,
        Stmt::AwaitMessage { .. } => K::AwaitMessage,
        Stmt::StreamMessage { .. } => K::StreamMessage,
        Stmt::MergeCrdt { .. } => K::MergeCrdt,
        Stmt::JoinCrdt { .. } => K::JoinCrdt,
        Stmt::IncreaseCrdt { .. } => K::IncreaseCrdt,
        Stmt::DecreaseCrdt { .. } => K::DecreaseCrdt,
        Stmt::AppendToSequence { .. } => K::AppendToSequence,
        Stmt::ResolveConflict { .. } => K::ResolveConflict,
        Stmt::Check { .. } => K::Check,
        Stmt::Listen { .. } => K::Listen,
        Stmt::Serve { .. } => K::Serve,
        Stmt::ConnectTo { .. } => K::ConnectTo,
        Stmt::LetPeerAgent { .. } => K::LetPeerAgent,
        Stmt::Sleep { .. } => K::Sleep,
        Stmt::Sync { .. } => K::Sync,
        Stmt::Ui(_) => K::Ui,
        Stmt::Fetch { .. } => K::Fetch,
        Stmt::Replay { .. } => K::Replay,
        Stmt::ServeFetches => K::ServeFetches,
        Stmt::GivenFetch { .. } => K::GivenFetch,
        Stmt::ComponentDef(_) => K::ComponentDef,
        Stmt::DirectiveDef { .. } => K::DirectiveDef,
        Stmt::RouteDef { .. } => K::RouteDef,
        Stmt::MiddlewareDef { .. } => K::MiddlewareDef,
        Stmt::RedirectDef { .. } => K::RedirectDef,
        Stmt::Mount { .. } => K::Mount,
        Stmt::LaunchTask { .. } => K::LaunchTask,
        Stmt::LaunchTaskWithHandle { .. } => K::LaunchTaskWithHandle,
        Stmt::CreatePipe { .. } => K::CreatePipe,
        Stmt::SendPipe { .. } => K::SendPipe,
        Stmt::ReceivePipe { .. } => K::ReceivePipe,
        Stmt::TrySendPipe { .. } => K::TrySendPipe,
        Stmt::TryReceivePipe { .. } => K::TryReceivePipe,
        Stmt::StopTask { .. } => K::StopTask,
        Stmt::CloseChannel { .. } => K::CloseChannel,
        Stmt::AwaitTask { .. } => K::AwaitTask,
        Stmt::Select { .. } => K::Select,
        Stmt::Theorem(_) => K::Theorem,
        Stmt::Definition(_) => K::Definition,
        Stmt::Axiom(_) => K::Axiom,
        Stmt::Theory(_) => K::Theory,
        Stmt::Escape { .. } => K::Escape,
        Stmt::Require { .. } => K::Require,
        Stmt::Site(_) => K::Site,
    }
}

pub const ALL_STMT_KINDS: &[StmtKind] = {
    use StmtKind::*;
    &[
        Let,
        Set,
        Swap,
        Call,
        If,
        While,
        Repeat,
        Return,
        Break,
        Assert,
        Trust,
        RuntimeAssert,
        Fail,
        Give,
        Show,
        SetField,
        StructDef,
        FunctionDef,
        TestDef,
        Expect,
        ExpectFail,
        ExpectOutput,
        AdvanceTime,
        GivenFile,
        Observe,
        During,
        ForAll,
        Inspect,
        Push,
        Pop,
        Add,
        Remove,
        SetIndex,
        Splice,
        Reversibly,
        Zone,
        Concurrent,
        Parallel,
        ReadFrom,
        WriteFile,
        Spawn,
        SendMessage,
        AwaitMessage,
        StreamMessage,
        MergeCrdt,
        JoinCrdt,
        IncreaseCrdt,
        DecreaseCrdt,
        AppendToSequence,
        ResolveConflict,
        Check,
        Listen,
        Serve,
        ConnectTo,
        LetPeerAgent,
        Sleep,
        Sync,
        Ui,
        Fetch,
        Replay,
        ServeFetches,
        GivenFetch,
        ComponentDef,
        DirectiveDef,
        RouteDef,
        MiddlewareDef,
        RedirectDef,
        Mount,
        LaunchTask,
        LaunchTaskWithHandle,
        CreatePipe,
        SendPipe,
        ReceivePipe,
        TrySendPipe,
        TryReceivePipe,
        StopTask,
        CloseChannel,
        AwaitTask,
        Select,
        Theorem,
        Definition,
        Axiom,
        Theory,
        Escape,
        Require,
        Site,
    ]
};

/// One kind per `Expr` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExprKind {
    Literal,
    Identifier,
    BinaryOp,
    Not,
    Call,
    Index,
    Slice,
    Copy,
    Give,
    Length,
    Contains,
    Union,
    Intersection,
    ManifestOf,
    ChunkAt,
    List,
    Tuple,
    Range,
    FieldAccess,
    New,
    NewVariant,
    Escape,
    OptionSome,
    OptionNone,
    WithCapacity,
    Closure,
    CallExpr,
    InterpolatedString,
    View,
    Dom,
}

/// ★ No `_` arm: a new `Expr` variant does not compile until it is classified.
pub fn classify_expr(e: &Expr<'_>) -> ExprKind {
    use ExprKind as K;
    match e {
        Expr::Literal(_) => K::Literal,
        Expr::Identifier(_) => K::Identifier,
        Expr::BinaryOp { .. } => K::BinaryOp,
        Expr::Not { .. } => K::Not,
        Expr::Call { .. } => K::Call,
        Expr::Index { .. } => K::Index,
        Expr::Slice { .. } => K::Slice,
        Expr::Copy { .. } => K::Copy,
        Expr::Give { .. } => K::Give,
        Expr::Length { .. } => K::Length,
        Expr::Contains { .. } => K::Contains,
        Expr::Union { .. } => K::Union,
        Expr::Intersection { .. } => K::Intersection,
        Expr::ManifestOf { .. } => K::ManifestOf,
        Expr::ChunkAt { .. } => K::ChunkAt,
        Expr::List(_) => K::List,
        Expr::Tuple(_) => K::Tuple,
        Expr::Range { .. } => K::Range,
        Expr::FieldAccess { .. } => K::FieldAccess,
        Expr::New { .. } => K::New,
        Expr::NewVariant { .. } => K::NewVariant,
        Expr::Escape { .. } => K::Escape,
        Expr::OptionSome { .. } => K::OptionSome,
        Expr::OptionNone => K::OptionNone,
        Expr::WithCapacity { .. } => K::WithCapacity,
        Expr::Closure { .. } => K::Closure,
        Expr::CallExpr { .. } => K::CallExpr,
        Expr::InterpolatedString(_) => K::InterpolatedString,
        Expr::View { .. } => K::View,
        Expr::Dom { .. } => K::Dom,
    }
}

pub const ALL_EXPR_KINDS: &[ExprKind] = {
    use ExprKind::*;
    &[
        Literal,
        Identifier,
        BinaryOp,
        Not,
        Call,
        Index,
        Slice,
        Copy,
        Give,
        Length,
        Contains,
        Union,
        Intersection,
        ManifestOf,
        ChunkAt,
        List,
        Tuple,
        Range,
        FieldAccess,
        New,
        NewVariant,
        Escape,
        OptionSome,
        OptionNone,
        WithCapacity,
        Closure,
        CallExpr,
        InterpolatedString,
        View,
        Dom,
    ]
};

/// ★ No `_` arm: a new binary operator does not compile until it is named here.
pub fn classify_binop(op: &BinaryOpKind) -> &'static str {
    match op {
        BinaryOpKind::Add => "Add",
        BinaryOpKind::Subtract => "Subtract",
        BinaryOpKind::Multiply => "Multiply",
        BinaryOpKind::Pow => "Pow",
        BinaryOpKind::Divide => "Divide",
        BinaryOpKind::ExactDivide => "ExactDivide",
        BinaryOpKind::FloorDivide => "FloorDivide",
        BinaryOpKind::Modulo => "Modulo",
        BinaryOpKind::Eq => "Eq",
        BinaryOpKind::NotEq => "NotEq",
        BinaryOpKind::Lt => "Lt",
        BinaryOpKind::Gt => "Gt",
        BinaryOpKind::LtEq => "LtEq",
        BinaryOpKind::GtEq => "GtEq",
        BinaryOpKind::And => "And",
        BinaryOpKind::Or => "Or",
        BinaryOpKind::Concat => "Concat",
        BinaryOpKind::SeqConcat => "SeqConcat",
        BinaryOpKind::ApproxEq => "ApproxEq",
        BinaryOpKind::BitXor => "BitXor",
        BinaryOpKind::BitAnd => "BitAnd",
        BinaryOpKind::BitOr => "BitOr",
        BinaryOpKind::Shl => "Shl",
        BinaryOpKind::Shr => "Shr",
    }
}

pub const ALL_BINOPS: &[BinaryOpKind] = &[
    BinaryOpKind::Add,
    BinaryOpKind::Subtract,
    BinaryOpKind::Multiply,
    BinaryOpKind::Pow,
    BinaryOpKind::Divide,
    BinaryOpKind::ExactDivide,
    BinaryOpKind::FloorDivide,
    BinaryOpKind::Modulo,
    BinaryOpKind::Eq,
    BinaryOpKind::NotEq,
    BinaryOpKind::Lt,
    BinaryOpKind::Gt,
    BinaryOpKind::LtEq,
    BinaryOpKind::GtEq,
    BinaryOpKind::And,
    BinaryOpKind::Or,
    BinaryOpKind::Concat,
    BinaryOpKind::SeqConcat,
    BinaryOpKind::ApproxEq,
    BinaryOpKind::BitXor,
    BinaryOpKind::BitAnd,
    BinaryOpKind::BitOr,
    BinaryOpKind::Shl,
    BinaryOpKind::Shr,
];

/// Every instruction of the VM's ISA, by the product's own name for it.
pub fn all_ops() -> BTreeSet<&'static str> {
    logicaffeine_compile::vm::Op::all_variants()
        .iter()
        .map(|o| o.name())
        .collect()
}

/// Every builtin the language has, by name, split by where it can exist: `(host, browser_only)`.
pub fn all_builtins() -> (BTreeSet<String>, BTreeSet<String>) {
    use logicaffeine_compile::semantics::builtins::BuiltinId;
    let mut host = BTreeSet::new();
    let mut browser = BTreeSet::new();
    for b in BuiltinId::ALL {
        if b.is_browser_only() {
            browser.insert(format!("{b:?}"));
        } else {
            host.insert(format!("{b:?}"));
        }
    }
    (host, browser)
}

/// Everything one program contains.
#[derive(Clone, Debug, Default)]
pub struct Reached {
    pub stmts: BTreeSet<StmtKind>,
    pub exprs: BTreeSet<ExprKind>,
    pub binops: BTreeSet<&'static str>,
    pub ops: BTreeSet<&'static str>,
    pub builtins: BTreeSet<String>,
}

/// What `source` contains: its statement, expression and operator arms from the parse, and its instructions
/// and builtins from the bytecode, unoptimized and optimized (the optimizer emits instructions of its own).
pub fn reached(source: &str) -> Result<Reached, String> {
    let mut r =
        logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, interner| {
            let (stmts, _types, _policies) = parsed?;
            let mut r = Reached::default();
            for_each_node(stmts, |node| match node {
                Visited::Stmt(s) => {
                    r.stmts.insert(classify_stmt(s));
                }
                Visited::Expr(e) => {
                    r.exprs.insert(classify_expr(e));
                    match e {
                        Expr::BinaryOp { op, .. } => {
                            r.binops.insert(classify_binop(op));
                        }
                        Expr::Call { function, args } => {
                            use logicaffeine_compile::semantics::builtins::{
                                builtin_from_name, system_native,
                            };
                            let name = interner.resolve(*function);
                            if let Some(b) = builtin_from_name(name)
                                .or_else(|| system_native(name, args.len()))
                            {
                                r.builtins.insert(format!("{b:?}"));
                            }
                        }
                        _ => {}
                    }
                }
            });
            Ok::<Reached, String>(r)
        })?;
    let programs = [
        logicaffeine_compile::compile::compile_to_program(source).ok(),
        std::panic::catch_unwind(|| {
            logicaffeine_compile::compile::compile_to_program_optimized(source).ok()
        })
        .ok()
        .flatten(),
    ];
    for program in programs.iter().flatten() {
        for op in &program.code {
            r.ops.insert(op.name());
            if let logicaffeine_compile::vm::Op::CallBuiltin { builtin, .. } = op {
                r.builtins.insert(format!("{builtin:?}"));
            }
        }
    }
    Ok(r)
}

/// How a statement or expression kind is exercised.
pub enum Probe {
    /// Standalone programs: every one runs on every backend.
    Programs(&'static [&'static str]),
    /// Programs whose `## Test` blocks carry the construct: every test runs on every backend that runs tests.
    Tests(&'static [&'static str]),
    /// Pages: mounted, rendered and driven on every backend that runs a page.
    Pages(&'static [&'static str]),
    /// A long-lived server: every backend serves it and answers the same requests with the same bytes.
    Servers(&'static [&'static str]),
    /// Programs that write the construct in source, which the front end lowers into other arms before any
    /// backend receives the program: the lock holds that the arm never survives parsing, so no backend can
    /// mean anything by it, and the program itself runs like any other.
    LoweredAway(&'static [&'static str]),
    /// Programs whose meaning exists on the Rust backends only (Tristen's ruling on `Escape to Rust:`): both
    /// Rust backends run them and agree with each other, and every other backend refuses them exactly as the VM
    /// does, so the refusal is one message everywhere.
    RustOnly(&'static [&'static str]),
    /// The construct exists but no program can reach it, or whether it may exist at all is the author's
    /// ruling. Either way the lock is red while any kind says this.
    Open(&'static str),
}

/// Every program is complete. A file statement names a relative path, which resolves in the run's own working
/// directory, so the text is the same on every backend and no run touches the tree.
pub fn stmt_probe(k: StmtKind) -> Probe {
    use Probe::{Open, Pages, Programs, RustOnly, Servers, Tests};
    use StmtKind as K;
    match k {
        K::Let => Programs(&["## Main\nLet x be 7.\nShow x.\n"]),
        K::Set => Programs(&["## Main\nLet mutable x be 1.\nSet x to 7.\nShow x.\n"]),
        K::Swap => Programs(&["## Main\nLet mutable a be 1.\nLet mutable b be 2.\nSwap a and b.\nShow a.\nShow b.\n"]),
        K::Call => Programs(&["## To greet (n: Int):\n    Show n.\n\n## Main\nCall greet with 7.\n"]),
        K::If => Programs(&["## Main\nLet x be 3.\nIf x is greater than 2:\n    Show \"big\".\nOtherwise:\n    Show \"small\".\n"]),
        K::While => Programs(&["## Main\nLet mutable i be 0.\nWhile i is less than 3:\n    Set i to i + 1.\nShow i.\n"]),
        K::Repeat => Programs(&["## Main\nLet mutable n be 0.\nRepeat for i from 1 to 4:\n    Set n to n + i.\nShow n.\n"]),
        K::Return => Programs(&["## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).\n"]),
        K::Break => Programs(&["## Main\nLet mutable i be 0.\nWhile i is less than 10:\n    Set i to i + 1.\n    If i is equal to 4:\n        Break.\nShow i.\n"]),
        K::RuntimeAssert => Programs(&[
            "## Main\nLet x be 42.\nAssert that x is equal to 42.\nShow x.\n",
            "## Main\nShow 1.\nLet x be 1.\nAssert that x is equal to 2.\nShow 2.\n",
        ]),
        K::Fail => Programs(&["## Main\nShow 1.\nFail with \"stop here\".\nShow 2.\n"]),
        K::Give => Programs(&["## To consume (xs: Seq of Int):\n    Show length of xs.\n\n## Main\nLet xs be [1, 2, 3].\nGive xs to consume.\n"]),
        K::Show => Programs(&["## Main\nShow 7.\nShow \"seven\".\nShow [1, 2].\nShow \"\".\nShow 2.5.\n"]),
        K::SetField => Programs(&["## A Point has:\n    An x: Int.\n\n## Main\nLet mutable p be a new Point with x 1.\nSet p's x to 5.\nShow p's x.\n"]),
        K::StructDef => Programs(&["## Store Scratch\nState:\n    Let n be 0.\n\n## Main\nShow 1.\n"]),
        K::FunctionDef => Programs(&["## To add (a: Int, b: Int) -> Int:\n    Return a + b.\n\n## Main\nShow add(3, 4).\n"]),
        K::Inspect => Programs(&["## Definition\nA Shape is either:\n    A Circle with a radius, which is Int.\n    A Point.\n\n## Main\nLet s be a new Circle with radius 3.\nInspect s:\n    When Circle (r):\n        Show r.\n    When Point:\n        Show \"point\".\n"]),
        K::Push => Programs(&["## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nShow xs.\n"]),
        K::Pop => Programs(&["## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nPop from xs into last.\nShow last.\nShow xs.\n"]),
        K::Add => Programs(&["## Main\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nAdd 1 to s.\nAdd 2 to s.\nShow length of s.\n"]),
        K::Remove => Programs(&["## Main\nLet mutable s be a new Set of Int.\nAdd 1 to s.\nAdd 7 to s.\nRemove 7 from s.\nShow length of s.\n"]),
        K::SetIndex => Programs(&[
            "## Main\nLet mutable xs be [1, 2, 3].\nSet item 2 of xs to 9.\nShow xs.\n",
            "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"alice\" of m to 30.\nSet item \"alice\" of m to 31.\nShow item \"alice\" of m.\n",
        ]),
        K::Splice => Programs(&["## Main\nLet xs be [1, 5, 2, 8, 3].\nLet small be every n in xs where n is less than 4.\nShow small.\n"]),
        K::Reversibly => Programs(&["## Main\nLet mutable x be 5.\nReversibly:\n    Set x to x + 3.\nShow x.\n"]),
        K::Zone => Programs(&["## Main\nInside a zone called \"Scratch\":\n    Let x be 5.\n    Show x.\nShow \"after\".\n"]),
        K::Concurrent => Programs(&["## To first -> Int:\n    Return 1.\n\n## To second -> Int:\n    Return 2.\n\n## Main\nAttempt all of the following:\n    Let a be first().\n    Let b be second().\nShow a + b.\n"]),
        K::Parallel => Programs(&["## To first -> Int:\n    Return 1.\n\n## To second -> Int:\n    Return 2.\n\n## Main\nSimultaneously:\n    Let a be first().\n    Let b be second().\nShow a + b.\n"]),
        K::ReadFrom => Programs(&["## Main\nWrite \"hello\" to file \"read.txt\".\nRead contents from file \"read.txt\".\nShow contents.\n"]),
        K::WriteFile => Programs(&["## Main\nWrite \"written\" to file \"write.txt\".\nRead contents from file \"write.txt\".\nShow contents.\n"]),
        K::Spawn => Programs(&["## Main\nSpawn a Worker called \"w1\".\nSend 7 to \"w1\".\nAwait response from \"w1\" into reply.\nShow reply.\n"]),
        K::SendMessage => Programs(&["## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend 7 to peer.\nAwait response from peer into got.\nShow got.\n"]),
        K::AwaitMessage => Programs(&["## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend \"hello\" to peer.\nAwait response from peer into got.\nShow got.\n", "## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend 41 to peer.\nAwait response from peer into got.\nLet n be got + 1.\nShow n.\n"]),
        K::StreamMessage => Programs(&["## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nLet vals be [1, 2, 3].\nStream vals to peer.\nAwait stream from peer into b.\nShow item 2 of b.\nShow length of b.\n"]),
        K::MergeCrdt => Programs(&["## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable a be a new Counter.\nIncrease a's points by 5.\nLet mutable b be a new Counter.\nIncrease b's points by 3.\nMerge b into a.\nShow a's points.\n"]),
        K::JoinCrdt => Programs(&["## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable a be a new Counter.\nIncrease a's points by 5.\nLet mutable b be a new Counter.\nIncrease b's points by 3.\nConverge b into a.\nConverge b into a.\nShow a's points.\n"]),
        K::IncreaseCrdt => Programs(&["## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n\n## Main\nLet mutable w be a new Wallet.\nIncrease w's balance by 3.\nShow w's balance.\n"]),
        K::DecreaseCrdt => Programs(&["## Definition\nA Wallet is Shared and has:\n    balance: Tally.\n\n## Main\nLet mutable w be a new Wallet.\nIncrease w's balance by 3.\nDecrease w's balance by 10.\nShow w's balance.\n"]),
        K::AppendToSequence => Programs(&["## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n## Main\nLet d be a new Document.\nAppend \"Hello\" to d's lines.\nAppend \"World\" to d's lines.\nShow length of d's lines.\n"]),
        K::ResolveConflict => Programs(&["## Definition\nA Cell is Shared and has:\n    a value, which is a Divergent Int.\n\n## Main\nLet x be a new Cell.\nResolve x's value to 42.\nShow x's value.\n"]),
        K::Check => Programs(&[
            "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that the u is admin.\nShow \"ok\".\n",
            "## Definition\nA User has:\n    a role, which is Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"guest\".\nCheck that the u is admin.\nShow \"ok\".\n",
        ]),
        K::Listen => Programs(&["## Main\nListen on \"room-42\".\nShow \"listening\".\n"]),
        K::ConnectTo => Programs(&["## Main\nConnect to \"ws://127.0.0.1:9\".\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend 5 to peer.\nAwait response from peer into got.\nShow got.\n"]),
        K::LetPeerAgent => Programs(&["## Main\nListen on \"me\".\nLet peer be a PeerAgent at \"me\".\nSend true to peer.\nAwait response from peer into got.\nShow got.\n"]),
        K::Sync => Programs(&["## Definition\nA Tally has:\n    n: a ConvergentCount.\n\n## Main\nLet mutable state be a new Tally.\nIncrease state's n by 3.\nSync state on \"topic\".\nShow state's n.\n"]),
        K::Sleep => Programs(&["## Main\nShow 1.\nSleep 5.\nShow 2.\n"]),
        K::LaunchTask => Programs(&["## To worker:\n    Show \"work\".\n\n## Main\nLaunch a task to worker.\nShow \"main\".\n"]),
        K::LaunchTaskWithHandle => Programs(&["## To slow:\n    Sleep 4.\n    Show \"slow\".\n\n## Main\nLet h be Launch a task to slow.\nAwait h.\nShow \"joined\".\n"]),
        K::CreatePipe => Programs(&["## Main\nLet jobs be a Pipe of Int.\nSend 10 into jobs.\nSend 20 into jobs.\nReceive a from jobs.\nReceive b from jobs.\nShow a + b.\n"]),
        K::SendPipe => Programs(&["## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Send 3 into ch.\n\n## Main\nLet jobs be a Pipe of Int.\nLaunch a task to produce with jobs.\nReceive a from jobs.\nReceive b from jobs.\nReceive c from jobs.\nShow a + b + c.\n"]),
        K::ReceivePipe => Programs(&["## Main\nLet ch be a Pipe of Int.\nSend 5 into ch.\nReceive x from ch.\nShow x.\n"]),
        K::TrySendPipe => Programs(&["## Main\nLet ch be a Pipe of Int.\nTry to send 42 into ch.\nReceive x from ch.\nShow x.\n"]),
        K::TryReceivePipe => Programs(&["## Main\nLet ch be a Pipe of Int.\nSend 7 into ch.\nTry to receive a from ch.\nShow a.\n"]),
        K::StopTask => Programs(&["## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n## Main\nLet ch be a Pipe of Int.\nLet h be Launch a task to blocker with ch.\nStop h.\nShow \"main\".\n"]),
        K::CloseChannel => Programs(&["## Main\nLet ch be a Pipe of Int.\nSend 42 into ch.\nClose ch.\nReceive x from ch.\nShow x.\n"]),
        K::AwaitTask => Programs(&["## To worker:\n    Show 3.\n\n## Main\nLet h be Launch a task to worker.\nWait for h.\nShow 4.\n"]),
        K::Select => Programs(&[
            "## To producer (ch: Int):\n    Send 5 into ch.\n\n## Main\nLet jobs be a Pipe of Int.\nLaunch a task to producer with jobs.\nAwait the first of:\n    Receive x from jobs:\n        Show x.\n    After 10 seconds:\n        Show 0.\n",
            "## Main\nLet ch be a Pipe of Int.\nAwait the first of:\n    Receive v from ch:\n        Show v.\n    After 5 seconds:\n        Show 42.\n",
        ]),
        K::Observe => Programs(&["## Main\nNote that starting the run.\nShow \"hi\".\n"]),
        K::During => Programs(&["## Main\nDuring processing:\n    Show 1.\n    Show 2.\nShow 3.\n"]),
        K::RouteDef => Programs(&["## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n"]),
        K::MiddlewareDef => Programs(&["## Before each request (request: Request) -> Option of Response:\n    Return none.\n\n## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nShow route(\"GET\", \"/hi\").\n"]),
        K::RedirectDef => Programs(&["## Redirect /old -> /new.\n\n## Route GET /new -> Text:\n    Return \"moved\".\n\n## Main\nShow routeStatus(\"GET\", \"/old\").\n"]),
        K::Serve => Servers(&["## Route GET /hi -> Text:\n    Return \"hello\".\n\n## Main\nServe on port {PORT}.\n"]),

        K::Theorem => Programs(&["## Theorem: Socrates\nGiven: Socrates is a man.\nGiven: Every man is mortal.\nProve: Socrates is mortal.\n\n## Main\nShow 1.\n"]),
        K::Definition => Programs(&["## Define\nx is a glorp if and only if x is shiny and x is round.\n\n## Main\nShow 1.\n"]),
        K::Axiom => Programs(&["## Axiom flip: for all a b, Cong(a, b, b, a).\n\n## Main\nShow 1.\n"]),
        K::Theory => Programs(&["## Theory Tarski\n\nAxiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).\n\nTheorem reflexivity: prove for all a b, Cong(a, b, a, b).\n\n## Main\nShow 1.\n"]),
        K::Assert => Open(
            "no sentence produces `Stmt::Assert`: the parser's `Assert` keyword builds `RuntimeAssert` \
             (parser/mod.rs `parse_assert_statement`), and no other site constructs it. A proof-layer assert needs a \
             surface (a logic sentence, as `Trust` has) or the variant goes.",
        ),
        K::Trust => Programs(&["## Main\nLet x be 10.\nTrust that x is greater than 0 because \"set to 10\".\nShow x.\n"]),
        K::Require => Programs(&["## Requires\n    The \"memchr\" crate version \"2\".\n\n## Main\nShow 1.\n"]),
        K::Escape => RustOnly(&["## Main\nShow 1.\nEscape to Rust:\n    println!(\"from rust\");\nShow 2.\n"]),

        K::TestDef => Tests(&["## To triple (n: Int) -> Int:\n    Return n * 3.\n\n## Main\nShow triple(2).\n\n## Test: triple multiplies by three\n\nExpect that triple(4) is equal to 12.\n"]),
        K::Expect => Tests(&["## Main\nShow 1.\n\n## Test: arithmetic holds\n\nExpect that 2 + 2 is equal to 4.\n"]),
        K::ExpectFail => Tests(&["## To checked (n: Int) -> Int:\n    If n is less than 0:\n        Fail with \"negative\".\n    Return n.\n\n## Main\nShow checked(1).\n\n## Test: a negative stops with its reason\n\nExpect this to fail with \"negative\":\n    Show checked(0 - 1).\n"]),
        K::ExpectOutput => Tests(&["## Main\nShow \"hello\".\n\n## Test: the answer prints\n\nExpect output:\n    Show \"hello\".\n"]),
        K::AdvanceTime => Tests(&["## Main\nShow 1.\n\n## Test: the clock moves when told\n\nLet before be now().\nAdvance time by 5 seconds.\nExpect that now() is greater than before.\n"]),
        K::GivenFile => Tests(&["## Main\nShow 1.\n\n## Test: a given file reads back\n\nGiven a file \"notes.txt\" containing \"hi\".\nRead contents from file \"notes.txt\".\nExpect that contents is equal to \"hi\".\n"]),
        K::ForAll => Tests(&["## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(1).\n\n## Test: doubling is adding a number to itself\n\nFor all integers a:\n    Expect that double(a) is equal to a + a.\n"]),

        K::Ui => Pages(&["## Page /\n\nState:\n    Let clicks be 0.\n\nActions:\n    To bump:\n        Set clicks to clicks + 1.\n\nView:\n    <button id=\"more\" on-click={bump}>Clicked {clicks} times</button>\n\n## Test: each click counts once\n\nClick \"#more\".\nExpect that the text of \"#more\" is equal to \"Clicked 1 times\".\n"]),
        K::Site => Pages(&["## Page /\n\nState:\n    Let clicks be 0.\n\nActions:\n    To bump:\n        Set clicks to clicks + 1.\n\nView:\n    <button id=\"more\" on-click={bump}>Clicked {clicks} times</button>\n\n## Test: an action runs\n\nClick \"#more\".\nExpect that the text of \"#more\" is equal to \"Clicked 1 times\".\n"]),
        K::ComponentDef => Pages(&["## Component Badge (label: Text)\n\nView:\n    <span class=\"badge\">{label}</span>\n\n## Page /\n\nView:\n    <Badge label=\"new\" />\n\n## Test: the badge shows its label\n\nExpect that the text of \".badge\" is equal to \"new\".\n"]),
        K::Mount => Programs(&["## Main\nMount counter at \"counter.journal\".\nShow counter.\n"]),
        K::Fetch => Pages(&["## Page /\n\nState:\n    Let greeting be \"\".\n\nActions:\n    To load:\n        Get \"/api/hi\" as Text, then loaded.\n    To loaded (answer: Result of Text and HttpError):\n        Inspect answer:\n            When Ok (text):\n                Set greeting to text.\n            When Err (e):\n                Set greeting to \"error\".\n\nView:\n    <button id=\"go\" on-click={load}>go</button>\n    <p id=\"out\">{greeting}</p>\n\n## Test: a fetch answers the page\n\nGiven that GET \"/api/hi\" answers 200 with \"hi\".\nClick \"#go\".\nExpect that the text of \"#out\" is equal to \"hi\".\n"]),
        K::GivenFetch => Pages(&["## Page /\n\nState:\n    Let greeting be \"\".\n\nActions:\n    To load:\n        Get \"/api/hi\" as Text, then loaded.\n    To loaded (answer: Result of Text and HttpError):\n        Inspect answer:\n            When Ok (text):\n                Set greeting to text.\n            When Err (e):\n                Set greeting to \"error\".\n\nView:\n    <button id=\"go\" on-click={load}>go</button>\n    <p id=\"out\">{greeting}</p>\n\n## Test: a given answer is what the fetch sees\n\nGiven that GET \"/api/hi\" answers 200 with \"given\".\nClick \"#go\".\nExpect that the text of \"#out\" is equal to \"given\".\n"]),
        K::ServeFetches => Pages(&["## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n## Route GET /api/todo -> Todo:\n    Return a new Todo with id 7 and text \"from the server\".\n\n## Component App\nState:\n    Let label be \"...\".\n    Let problem be \"\".\nActions:\n    When the page loads:\n        Get \"/api/todo\" as Todo, then loaded.\n    To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n            When Ok (v):\n                Set label to v's text.\n            When Err (e):\n                Set problem to e's message.\nView:\n    <p id=\"label\">{label}</p>\n    <p id=\"problem\">{problem}</p>\n\n## Main\nServe fetches with routes.\nMount App.\nShow the text of \"#label\".\n"]),
        K::Replay => Pages(&["## Component App\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n\n## Main\nMount App.\nReplay these actions: bump.\nShow the text of \"#n\".\n"]),
        K::DirectiveDef => Pages(&["## Directive tooltip (el: Element) and (text: Text):\n    Call setAttribute with el and \"title\" and text.\n\n## Component App\nView:\n    <p tooltip={\"hint\"}>x</p>\n\n## Main\nMount App.\n"]),
    }
}

pub fn expr_probe(k: ExprKind) -> Probe {
    use ExprKind as K;
    use Probe::{LoweredAway, Open, Pages, Programs, RustOnly};
    match k {
        K::Literal => Programs(&["## Main\nShow 7.\nShow 2.5.\nShow \"text\".\nShow true.\n"]),
        K::Identifier => Programs(&["## Main\nLet x be 7.\nShow x.\n"]),
        K::BinaryOp => Programs(&["## Main\nShow 6 * 7.\n"]),
        K::Not => Programs(&["## Main\nShow not (1 is greater than 2).\n"]),
        K::Call => Programs(&["## Main\nShow upper(\"hi\").\n"]),
        K::Index => Programs(&["## Main\nShow item 2 of [10, 20, 30].\n"]),
        K::Slice => Programs(&["## Main\nShow sumSeq(items 1 through 2 of [1, 2, 3]).\n"]),
        K::Copy => Programs(&["## Main\nLet xs be [1, 2].\nLet ys be copy of xs.\nShow length of ys.\n"]),
        K::Give => Open(
            "no working sentence produces `Expr::Give`: only `parse_call_arg` builds it, a parenthesized call never \
             reaches that path, and no program, module, example or test uses the `Call f with give x.` form",
        ),
        K::Length => Programs(&["## Main\nShow length of [1, 2, 3].\n"]),
        K::Contains => Programs(&["## Main\nShow [1, 2, 3] contains 2.\n"]),
        K::Union => Programs(&["## Main\nLet xs be a new Set of Int.\nAdd 1 to xs.\nLet ys be a new Set of Int.\nAdd 2 to ys.\nShow length of (xs union ys).\n"]),
        K::Intersection => Programs(&["## Main\nLet xs be a new Set of Int.\nAdd 1 to xs.\nLet ys be a new Set of Int.\nAdd 1 to ys.\nShow length of (xs intersection ys).\n"]),
        K::List => Programs(&["## Main\nShow sumSeq([1, 2, 3]).\n"]),
        K::Tuple => Programs(&["## To addpair (p: Pair of Int and Int) -> Int:\n    Return item 1 of p + item 2 of p.\n\n## Main\nLet p be (3, 4).\nShow addpair(p).\n"]),
        K::Range => Programs(&["## Main\nLet mutable n be 0.\nRepeat for i from 1 to 3:\n    Set n to n + i.\nShow n.\n"]),
        K::FieldAccess | K::New => Programs(&["## A Row has:\n    a n (Int).\n\n## Main\nLet r be a new Row with n 7.\nShow r's n.\n"]),
        K::NewVariant => Programs(&["## To f (n: Int) -> Result of Int and Text:\n    Return a new Ok with value n.\n\n## Main\nInspect f(7):\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n"]),
        K::InterpolatedString => Programs(&["## Main\nLet n be 7.\nShow \"n is {n}\".\n"]),
        K::Closure => Programs(&["## Main\nLet f be (x: Int) -> x * 2.\nShow f(21).\n"]),
        K::CallExpr => Open(
            "no sentence produces `Expr::CallExpr`: every call, a closure's included, parses to `Expr::Call`, and \
             the variant is built only by hand in unit tests",
        ),
        K::WithCapacity => Programs(&["## Main\nLet a be a new Seq of Int with capacity 4.\nShow length of a.\n"]),
        K::OptionSome => Programs(&["## Main\nLet v be some 0.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        Show \"none\".\n"]),
        K::OptionNone => Programs(&["## Main\nLet v be none.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        Show \"none\".\n"]),
        K::ManifestOf => Programs(&["## Main\nInside a new zone called \"Z\":\n    Let m be the manifest of Z.\n    Show length of m.\n"]),
        K::View => LoweredAway(&["## To badge (label: Text) -> View:\n    <span>{label}</span>\n\n## Component App\nView:\n    <p>{badge(\"x\")}</p>\n\n## Main\nMount App.\n"]),
        K::Dom => Pages(&["## Page /\n\nState:\n    Let seen be \"\".\n\nActions:\n    To look:\n        Set seen to the text of \"#v\".\n\nView:\n    <p id=\"v\">view</p>\n    <button id=\"b\" on-click={look}>look</button>\n    <p id=\"out\">{seen}</p>\n\n## Test: a DOM read answers the page\n\nClick \"#b\".\nExpect that the text of \"#out\" is equal to \"view\".\n"]),
        K::Escape => RustOnly(&["## Main\nLet x: Int be Escape to Rust:\n    42_i64\nShow x.\n"]),
        K::ChunkAt => Programs(&["## Main\nInside a new zone called \"Z\":\n    Let c be the chunk at 1 in Z.\n    Show \"ok\".\n"]),
    }
}

/// A program per builtin that no statement or expression probe calls, each deterministic and self-contained:
/// the builtin's own call form, since several sentence forms (`length of xs`, `copy of xs`) compile to
/// dedicated instructions and never reach the builtin at all.
pub const BUILTIN_PROBES: &[(&str, &str)] = &[
    ("AddSeconds", "## Main\nShow format_timestamp(add_seconds(parse_timestamp(\"2024-03-15T10:30:45Z\"), 3600)).\n"),
    ("Ceil", "## Main\nShow ceil(3.2).\n"),
    ("Complex", "## Main\nShow complex(0, 1).\n"),
    ("CompressBytes", "## Main\nLet frame be [72, 72, 72, 72, 72, 72, 72, 72, 72, 72, 72, 72].\nLet packed be compressBytes(frame).\nLet restored be decompressBytes(packed).\nShow restored.\n"),
    ("Conjugate", "## Main\nShow conjugate(complex(3, 4)).\n"),
    ("Convert", "## Main\nShow convert(quantity(2, \"inch\"), \"centimeter\").\n"),
    ("Copy", "## Main\nLet a be [1, 2, 3].\nLet b be copy(a).\nPush 99 to a.\nShow b.\n"),
    ("DateOf", "## Main\nShow date_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("DayOf", "## Main\nShow day_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("EnvGet", "## Main\nInspect get(\"NO_COLOR\"):\n    When Some (v):\n        Show \"set to \" + v.\n    When None:\n        Show \"unset\".\n"),
    ("FecProtect", "## Main\nLet payload be [72, 101, 108, 108, 111, 33].\nLet packets be fecProtect(payload, 3, 2).\nLet a be item 2 of packets.\nLet b be item 3 of packets.\nLet c be item 5 of packets.\nLet received be [nothing, a, b, nothing, c].\nShow fecRestore(received, 3).\n"),
    ("FileRead", "## Main\nLet w be write(\"probe.txt\", \"hello\").\nLet r be read(\"probe.txt\").\nInspect r:\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n"),
    ("Floor", "## Main\nShow floor(3.7).\n"),
    ("FormatTimestamp", "## Main\nShow format_timestamp(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("FromWireBytes", "## Main\nLet v be [1, 2, 3].\nLet packed be wireBytes(v).\nShow fromWireBytes(packed).\n"),
    ("HourOf", "## Main\nShow hour_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("ImagPart", "## Main\nShow im(complex(3.0, 4.0)).\n"),
    ("InZone", "## Main\nShow in_zone(parse_timestamp(\"2024-07-01T12:00:00Z\"), \"America/New_York\").\n"),
    ("IntOfWord8", "## Main\nShow intOfWord8(word8(300)).\n"),
    ("Lanes4Word32Make", "## Main\nLet s be [1, 2, 3, 4].\nShow lanes4Word32(s).\n"),
    ("LeadingZeros", "## Main\nShow leading_zeros(1).\n"),
    ("Length", "## Main\nShow length([1, 2, 3]).\n"),
    ("LocalInstant", "## Main\nShow the hour of timestamp \"2024-07-01T12:00:00Z\" in \"America/New_York\".\n"),
    ("MakeDate", "## Main\nShow makeDate(2024, 3, 15).\n"),
    ("MakeMoment", "## Main\nShow makeMoment(2024, 3, 15, 10, 30, 45).\n"),
    ("MapOf", "## Main\nLet m be mapOf(\"a\", 1, \"b\", 2).\nShow length of m.\n"),
    ("MinuteOf", "## Main\nShow minute_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("Modular", "## Main\nShow modular(10, 7).\n"),
    ("Money", "## Main\nShow money(10, \"USD\").\n"),
    ("MonthOf", "## Main\nShow month_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("MonthsBetween", "## Main\nLet m be parse_timestamp(\"2020-01-15T00:00:00Z\").\nLet n be parse_timestamp(\"2024-07-15T00:00:00Z\").\nShow months_between(m, n).\n"),
    ("OpenBox", "## Main\nLet key be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32].\nLet box be sealBox(key, [72, 105]).\nShow openBox(key, box).\n"),
    ("OpenBytes", "## Main\nLet key be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32].\nLet nonce be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].\nLet plaintext be [72, 105].\nLet sealed be sealBytes(key, nonce, plaintext).\nShow openBytes(key, nonce, sealed).\n"),
    ("OpenFrame", "## Main\nLet key be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32].\nLet nonce be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].\nLet roster be [\"alice\", \"bob\", \"carol\", \"dave\"].\nLet sealed be sealFrame(key, nonce, roster).\nShow openFrame(key, nonce, sealed).\n"),
    ("OpenValueBox", "## Main\nLet key be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32].\nLet roster be [\"alice\", \"bob\"].\nShow openValueBox(key, sealValueBox(key, roster)).\n"),
    ("ParseTimestamp", "## Main\nShow parse_timestamp(\"2024-03-15T10:30:45Z\").\n"),
    ("Pow", "## Main\nShow pow(2, 10).\n"),
    ("Quantity", "## Main\nShow quantity(2, \"inch\").\n"),
    ("QuarterOf", "## Main\nShow quarter_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("RandomFloat", "## Main\nLet x be randomFloat().\nShow x >= 0.0 and x < 1.0.\n"),
    ("ReadBytes", "## Main\nLet w be write(\"probe.txt\", \"AB\").\nShow readBytes(\"probe.txt\").\n"),
    ("ReadReals", "## Main\nLet w be write(\"probe.f64\", \"AAAAAAAA\").\nShow readReals(\"probe.f64\").\n"),
    ("RealPart", "## Main\nShow re(complex(3.0, 4.0)).\n"),
    ("ReedSolomonEncode", "## Main\nLet d1 be [10, 20, 30].\nLet d2 be [40, 50, 60].\nLet data be [d1, d2].\nShow reedSolomonEncode(data, 2).\n"),
    ("ReedSolomonRecover", "## Main\nLet d1 be [65, 66, 67].\nLet d2 be [68, 69, 70].\nLet data be [d1, d2].\nLet encoded be reedSolomonEncode(data, 2).\nLet a be item 2 of encoded.\nLet b be item 4 of encoded.\nLet received be [nothing, a, nothing, b].\nShow reedSolomonRecover(received, 2).\n"),
    ("RepeatSeq", "## Main\nShow repeatSeq(7, 5).\n"),
    ("Round", "## Main\nShow round(2.5).\n"),
    ("SealBox", "## Main\nLet key be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32].\nLet box be sealBox(key, [1, 2, 3]).\nShow length of box.\nShow openBox(key, box).\n"),
    ("SealValueBox", "## Main\nLet key be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32].\nShow openValueBox(key, sealValueBox(key, 42)).\n"),
    ("SecondOf", "## Main\nShow second_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("SecondsBetween", "## Main\nLet m be parse_timestamp(\"2024-03-15T10:30:45Z\").\nLet n be parse_timestamp(\"2024-03-20T14:30:45Z\").\nShow seconds_between(m, n).\n"),
    ("SetOf", "## Main\nLet s be setOf(1, 2, 2, 3).\nShow length of s.\n"),
    ("SetRate", "## Main\nCall set_rate with \"USD\" and 1.\nCall set_rate with \"EUR\" and 2.\nShow to_currency(money(10, \"USD\"), \"EUR\").\n"),
    ("SetRates", "## Main\nLet mutable rates be a new Map of Text to Decimal.\nSet item \"USD\" of rates to decimal(\"1\").\nSet item \"EUR\" of rates to decimal(\"2\").\nCall set_rates with rates.\nShow to_currency(money(10, \"USD\"), \"EUR\").\n"),
    ("TimeOf", "## Main\nShow time_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("TimeNow", "## Main\nShow now() is greater than 1700000000000.\n"),
    ("TrailingZeros", "## Main\nShow trailing_zeros(8).\n"),
    ("Uuid", "## Main\nShow uuid(\"550e8400-e29b-41d4-a716-446655440000\").\n"),
    ("UuidDns", "## Main\nShow uuid_dns().\n"),
    ("UuidMax", "## Main\nShow uuid_max().\n"),
    ("UuidNil", "## Main\nShow uuid_nil().\n"),
    ("UuidOid", "## Main\nShow uuid_oid().\n"),
    ("UuidUrl", "## Main\nShow uuid_url().\n"),
    ("UuidVersion", "## Main\nShow uuid_version(uuid(\"550e8400-e29b-41d4-a716-446655440000\")).\n"),
    ("UuidX500", "## Main\nShow uuid_x500().\n"),
    ("VfsMount", "## Main\nWrite \"7\" to file \"c.journal\".\nMount c at \"c.journal\".\nShow c.\n"),
    ("WeekOf", "## Main\nShow week_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("WeekdayOf", "## Main\nShow weekday_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("Word8Make", "## Main\nShow word8(300).\n"),
    ("WriteWireResidual", "## Main\nShow writeWireResidual(\"hello\").\n"),
    ("YearOf", "## Main\nShow year_of(parse_timestamp(\"2024-03-15T10:30:45Z\")).\n"),
    ("YearsBetween", "## Main\nLet m be parse_timestamp(\"2020-01-15T00:00:00Z\").\nLet n be parse_timestamp(\"2024-07-15T00:00:00Z\").\nShow years_between(m, n).\n"),
];

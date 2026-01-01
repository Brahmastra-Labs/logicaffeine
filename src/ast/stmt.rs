use super::logic::LogicExpr;
use crate::intern::Symbol;

/// Type expression for explicit type annotations.
///
/// Represents type syntax like:
/// - `Int` → Primitive(Int)
/// - `User` → Named(User)
/// - `List of Int` → Generic { base: List, params: [Primitive(Int)] }
/// - `List of List of Int` → Generic { base: List, params: [Generic { base: List, params: [Primitive(Int)] }] }
/// - `Result of Int and Text` → Generic { base: Result, params: [Primitive(Int), Primitive(Text)] }
#[derive(Debug, Clone)]
pub enum TypeExpr<'a> {
    /// Primitive type: Int, Nat, Text, Bool
    Primitive(Symbol),
    /// Named type (user-defined): User, Point
    Named(Symbol),
    /// Generic type: List of Int, Option of Text, Result of Int and Text
    Generic {
        base: Symbol,
        params: &'a [TypeExpr<'a>],
    },
    /// Function type: fn(A, B) -> C (for higher-order functions)
    Function {
        inputs: &'a [TypeExpr<'a>],
        output: &'a TypeExpr<'a>,
    },
    /// Phase 43C: Refinement type with predicate constraint
    /// Example: `Int where it > 0`
    Refinement {
        /// The base type being refined
        base: &'a TypeExpr<'a>,
        /// The bound variable (usually "it")
        var: Symbol,
        /// The predicate constraint (from Logic Kernel)
        predicate: &'a LogicExpr<'a>,
    },
    /// Phase 53: Persistent storage wrapper type
    /// Example: `Persistent Counter`
    /// Semantics: Wraps a Shared type with journal-backed storage
    Persistent {
        /// The inner type (must be a Shared/CRDT type)
        inner: &'a TypeExpr<'a>,
    },
}

/// Phase 10: Source for Read statements
#[derive(Debug, Clone, Copy)]
pub enum ReadSource<'a> {
    /// Read from console (stdin)
    Console,
    /// Read from file at given path
    File(&'a Expr<'a>),
}

/// Binary operation kinds for imperative expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOpKind {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    // Grand Challenge: Logical operators for compound conditions
    And,
    Or,
    // Phase 53: String concatenation ("X combined with Y")
    Concat,
}

/// Block is a sequence of statements.
pub type Block<'a> = &'a [Stmt<'a>];

/// Phase 33: Match arm for Inspect statement
#[derive(Debug)]
pub struct MatchArm<'a> {
    pub enum_name: Option<Symbol>,          // The enum type (e.g., Shape)
    pub variant: Option<Symbol>,            // None = Otherwise (wildcard)
    pub bindings: Vec<(Symbol, Symbol)>,    // (field_name, binding_name)
    pub body: Block<'a>,
}

/// Imperative statement AST (LOGOS §15.0.0).
///
/// Stmt is the primary AST node for imperative code blocks like `## Main`
/// and function bodies. The Assert variant bridges to the Logic Kernel.
#[derive(Debug)]
pub enum Stmt<'a> {
    /// Variable binding: `Let x be 5.` or `Let x: Int be 5.`
    Let {
        var: Symbol,
        ty: Option<&'a TypeExpr<'a>>,
        value: &'a Expr<'a>,
        mutable: bool,
    },

    /// Mutation: `Set x to 10.`
    Set {
        target: Symbol,
        value: &'a Expr<'a>,
    },

    /// Function call as statement: `Call process with data.`
    Call {
        function: Symbol,
        args: Vec<&'a Expr<'a>>,
    },

    /// Conditional: `If condition: ... Otherwise: ...`
    If {
        cond: &'a Expr<'a>,
        then_block: Block<'a>,
        else_block: Option<Block<'a>>,
    },

    /// Loop: `While condition: ...` or `While condition (decreasing expr): ...`
    While {
        cond: &'a Expr<'a>,
        body: Block<'a>,
        /// Phase 44: Optional decreasing variant for termination proof
        decreasing: Option<&'a Expr<'a>>,
    },

    /// Iteration: `Repeat for x in list: ...` or `Repeat for i from 1 to 10: ...`
    Repeat {
        var: Symbol,
        iterable: &'a Expr<'a>,
        body: Block<'a>,
    },

    /// Return: `Return x.` or `Return.`
    Return {
        value: Option<&'a Expr<'a>>,
    },

    /// Bridge to Logic Kernel: `Assert that P.`
    Assert {
        proposition: &'a LogicExpr<'a>,
    },

    /// Phase 35: Documented assertion with justification
    /// `Trust that P because "reason".`
    /// Semantics: Documented runtime check that could be verified statically.
    Trust {
        proposition: &'a LogicExpr<'a>,
        justification: Symbol,
    },

    /// Runtime assertion with imperative condition
    /// `Assert that condition.` (for imperative mode)
    RuntimeAssert {
        condition: &'a Expr<'a>,
    },

    /// Ownership transfer (move): `Give x to processor.`
    /// Semantics: Move ownership of `object` to `recipient`.
    Give {
        object: &'a Expr<'a>,
        recipient: &'a Expr<'a>,
    },

    /// Immutable borrow: `Show x to console.`
    /// Semantics: Immutable borrow of `object` passed to `recipient`.
    Show {
        object: &'a Expr<'a>,
        recipient: &'a Expr<'a>,
    },

    /// Phase 31: Field mutation: `Set p's x to 10.`
    SetField {
        object: &'a Expr<'a>,
        field: Symbol,
        value: &'a Expr<'a>,
    },

    /// Phase 31: Struct definition for codegen
    /// Phase 47: Added is_portable for serde derives
    StructDef {
        name: Symbol,
        fields: Vec<(Symbol, Symbol, bool)>, // (name, type_name, is_public)
        is_portable: bool,                    // Phase 47: Derives Serialize/Deserialize
    },

    /// Phase 32/38: Function definition
    /// Phase 38: Updated for native functions and TypeExpr types
    FunctionDef {
        name: Symbol,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>, // Phase 38: Changed to TypeExpr
        body: Block<'a>,
        return_type: Option<&'a TypeExpr<'a>>,   // Phase 38: Changed to TypeExpr
        is_native: bool,                          // Phase 38: Native function flag
    },

    /// Phase 33: Pattern matching on sum types
    Inspect {
        target: &'a Expr<'a>,
        arms: Vec<MatchArm<'a>>,
        has_otherwise: bool,            // For exhaustiveness tracking
    },

    /// Phase 43D: Push to collection: `Push x to items.`
    Push {
        value: &'a Expr<'a>,
        collection: &'a Expr<'a>,
    },

    /// Phase 43D: Pop from collection: `Pop from items.` or `Pop from items into y.`
    Pop {
        collection: &'a Expr<'a>,
        into: Option<Symbol>,
    },

    /// Add to set: `Add x to set.`
    Add {
        value: &'a Expr<'a>,
        collection: &'a Expr<'a>,
    },

    /// Remove from set: `Remove x from set.`
    Remove {
        value: &'a Expr<'a>,
        collection: &'a Expr<'a>,
    },

    /// Index assignment: `Set item N of X to Y.`
    SetIndex {
        collection: &'a Expr<'a>,
        index: &'a Expr<'a>,
        value: &'a Expr<'a>,
    },

    /// Phase 8.5: Memory arena block (Zone)
    /// "Inside a new zone called 'Scratch':"
    /// "Inside a zone called 'Buffer' of size 1 MB:"
    /// "Inside a zone called 'Data' mapped from 'file.bin':"
    Zone {
        /// The variable name for the arena handle (e.g., "Scratch")
        name: Symbol,
        /// Optional pre-allocated capacity in bytes (Heap zones only)
        capacity: Option<usize>,
        /// Optional file path for memory-mapped zones (Mapped zones only)
        source_file: Option<Symbol>,
        /// The code block executed within this memory context
        body: Block<'a>,
    },

    /// Phase 9: Concurrent execution block (async, I/O-bound)
    /// "Attempt all of the following:"
    /// Semantics: All tasks run concurrently via tokio::join!
    /// Best for: network requests, file I/O, waiting operations
    Concurrent {
        /// The statements to execute concurrently
        tasks: Block<'a>,
    },

    /// Phase 9: Parallel execution block (CPU-bound)
    /// "Simultaneously:"
    /// Semantics: True parallelism via rayon::join or thread::spawn
    /// Best for: computation, data processing, number crunching
    Parallel {
        /// The statements to execute in parallel
        tasks: Block<'a>,
    },

    /// Phase 10: Read from console or file
    /// `Read input from the console.` or `Read data from file "path.txt".`
    ReadFrom {
        var: Symbol,
        source: ReadSource<'a>,
    },

    /// Phase 10: Write to file
    /// `Write "content" to file "output.txt".`
    WriteFile {
        content: &'a Expr<'a>,
        path: &'a Expr<'a>,
    },

    /// Phase 46: Spawn an agent
    /// `Spawn a Worker called "w1".`
    Spawn {
        agent_type: Symbol,
        name: Symbol,
    },

    /// Phase 46: Send message to agent
    /// `Send Ping to "agent".`
    SendMessage {
        message: &'a Expr<'a>,
        destination: &'a Expr<'a>,
    },

    /// Phase 46: Await response from agent
    /// `Await response from "agent" into result.`
    AwaitMessage {
        source: &'a Expr<'a>,
        into: Symbol,
    },

    /// Phase 49: Merge CRDT state
    /// `Merge remote into local.` or `Merge remote's field into local's field.`
    MergeCrdt {
        source: &'a Expr<'a>,
        target: &'a Expr<'a>,
    },

    /// Phase 49: Increment GCounter
    /// `Increase local's points by 10.`
    IncreaseCrdt {
        object: &'a Expr<'a>,
        field: Symbol,
        amount: &'a Expr<'a>,
    },

    /// Phase 50: Security check - mandatory runtime guard
    /// `Check that user is admin.`
    /// `Check that user can publish the document.`
    /// Semantics: NEVER optimized out. Panics if condition is false.
    Check {
        /// The subject being checked (e.g., "user")
        subject: Symbol,
        /// The predicate name (e.g., "admin") or action (e.g., "publish")
        predicate: Symbol,
        /// True if this is a capability check (can [action])
        is_capability: bool,
        /// For capabilities: the object being acted on (e.g., "document")
        object: Option<Symbol>,
        /// Original English text for error message
        source_text: String,
        /// Source location for error reporting
        span: crate::token::Span,
    },

    /// Phase 51: Listen on network address
    /// `Listen on "/ip4/127.0.0.1/tcp/8000".`
    /// Semantics: Bind to address, start accepting connections via libp2p
    Listen {
        address: &'a Expr<'a>,
    },

    /// Phase 51: Connect to remote peer
    /// `Connect to "/ip4/127.0.0.1/tcp/8000".`
    /// Semantics: Dial peer via libp2p
    ConnectTo {
        address: &'a Expr<'a>,
    },

    /// Phase 51: Create PeerAgent remote handle
    /// `Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000".`
    /// Semantics: Create handle for remote agent communication
    LetPeerAgent {
        var: Symbol,
        address: &'a Expr<'a>,
    },

    /// Phase 51: Sleep for milliseconds
    /// `Sleep 1000.` or `Sleep delay.`
    /// Semantics: Pause execution for N milliseconds (async)
    Sleep {
        milliseconds: &'a Expr<'a>,
    },

    /// Phase 52: Sync CRDT variable on topic
    /// `Sync x on "topic".`
    /// Semantics: Subscribe to GossipSub topic, auto-publish on mutation, auto-merge on receive
    Sync {
        var: Symbol,
        topic: &'a Expr<'a>,
    },

    /// Phase 53: Mount persistent CRDT from journal file
    /// `Mount counter at "data/counter.journal".`
    /// Semantics: Load or create journal, replay operations to reconstruct state
    Mount {
        /// The variable name for the mounted value
        var: Symbol,
        /// The path expression for the journal file
        path: &'a Expr<'a>,
    },

    // =========================================================================
    // Phase 54: Go-like Concurrency (Green Threads, Channels, Select)
    // =========================================================================

    /// Phase 54: Launch a fire-and-forget task (green thread)
    /// `Launch a task to process(data).`
    /// Semantics: tokio::spawn with no handle capture
    LaunchTask {
        /// The function to call
        function: Symbol,
        /// Arguments to pass
        args: Vec<&'a Expr<'a>>,
    },

    /// Phase 54: Launch a task with handle for control
    /// `Let worker be Launch a task to process(data).`
    /// Semantics: tokio::spawn returning JoinHandle
    LaunchTaskWithHandle {
        /// Variable to bind the handle
        handle: Symbol,
        /// The function to call
        function: Symbol,
        /// Arguments to pass
        args: Vec<&'a Expr<'a>>,
    },

    /// Phase 54: Create a bounded channel (pipe)
    /// `Let jobs be a new Pipe of Int.`
    /// Semantics: tokio::sync::mpsc::channel(32)
    CreatePipe {
        /// Variable for the pipe
        var: Symbol,
        /// Type of values in the pipe
        element_type: Symbol,
        /// Optional capacity (defaults to 32)
        capacity: Option<u32>,
    },

    /// Phase 54: Blocking send into pipe
    /// `Send value into pipe.`
    /// Semantics: pipe_tx.send(value).await
    SendPipe {
        /// The value to send
        value: &'a Expr<'a>,
        /// The pipe to send into
        pipe: &'a Expr<'a>,
    },

    /// Phase 54: Blocking receive from pipe
    /// `Receive x from pipe.`
    /// Semantics: let x = pipe_rx.recv().await
    ReceivePipe {
        /// Variable to bind the received value
        var: Symbol,
        /// The pipe to receive from
        pipe: &'a Expr<'a>,
    },

    /// Phase 54: Non-blocking send (try)
    /// `Try to send value into pipe.`
    /// Semantics: pipe_tx.try_send(value) - returns immediately
    TrySendPipe {
        /// The value to send
        value: &'a Expr<'a>,
        /// The pipe to send into
        pipe: &'a Expr<'a>,
        /// Variable to bind the result (true/false)
        result: Option<Symbol>,
    },

    /// Phase 54: Non-blocking receive (try)
    /// `Try to receive x from pipe.`
    /// Semantics: pipe_rx.try_recv() - returns Option
    TryReceivePipe {
        /// Variable to bind the received value (if any)
        var: Symbol,
        /// The pipe to receive from
        pipe: &'a Expr<'a>,
    },

    /// Phase 54: Cancel a spawned task
    /// `Stop worker.`
    /// Semantics: handle.abort()
    StopTask {
        /// The handle to cancel
        handle: &'a Expr<'a>,
    },

    /// Phase 54: Select on multiple channels/timeouts
    /// `Await the first of:`
    ///     `Receive x from ch:`
    ///         `...`
    ///     `After 5 seconds:`
    ///         `...`
    /// Semantics: tokio::select! with auto-cancel
    Select {
        /// The branches to select from
        branches: Vec<SelectBranch<'a>>,
    },
}

/// Phase 54: A branch in a Select statement
#[derive(Debug)]
pub enum SelectBranch<'a> {
    /// Receive from a pipe: `Receive x from ch:`
    Receive {
        var: Symbol,
        pipe: &'a Expr<'a>,
        body: Block<'a>,
    },
    /// Timeout: `After N seconds:` or `After N milliseconds:`
    Timeout {
        milliseconds: &'a Expr<'a>,
        body: Block<'a>,
    },
}

/// Shared expression type for pure computations (LOGOS §15.0.0).
///
/// Expr is used by both LogicExpr (as terms) and Stmt (as values).
/// These are pure computations without side effects.
#[derive(Debug)]
pub enum Expr<'a> {
    /// Literal value: 42, "hello", true, nothing
    Literal(Literal),

    /// Variable reference: x
    Identifier(Symbol),

    /// Binary operation: x plus y
    BinaryOp {
        op: BinaryOpKind,
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
    },

    /// Function call as expression: f(x, y)
    Call {
        function: Symbol,
        args: Vec<&'a Expr<'a>>,
    },

    /// Phase 43D: Dynamic index access: `items at i` (1-indexed)
    Index {
        collection: &'a Expr<'a>,
        index: &'a Expr<'a>,
    },

    /// Phase 43D: Dynamic slice access: `items 1 through mid` (1-indexed, inclusive)
    Slice {
        collection: &'a Expr<'a>,
        start: &'a Expr<'a>,
        end: &'a Expr<'a>,
    },

    /// Phase 43D: Copy expression: `copy of slice` → slice.to_vec()
    Copy {
        expr: &'a Expr<'a>,
    },

    /// Phase 43D: Length expression: `length of items` → items.len()
    Length {
        collection: &'a Expr<'a>,
    },

    /// Set contains: `set contains x` or `x in set`
    Contains {
        collection: &'a Expr<'a>,
        value: &'a Expr<'a>,
    },

    /// Set union: `a union b`
    Union {
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
    },

    /// Set intersection: `a intersection b`
    Intersection {
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
    },

    /// Phase 48: Get manifest of a zone
    /// `the manifest of Zone` → FileSipper::from_zone(&zone).manifest()
    ManifestOf {
        zone: &'a Expr<'a>,
    },

    /// Phase 48: Get chunk at index from a zone
    /// `the chunk at N in Zone` → FileSipper::from_zone(&zone).get_chunk(N)
    ChunkAt {
        index: &'a Expr<'a>,
        zone: &'a Expr<'a>,
    },

    /// List literal: [1, 2, 3]
    List(Vec<&'a Expr<'a>>),

    /// Range: 1 to 10 (inclusive)
    Range {
        start: &'a Expr<'a>,
        end: &'a Expr<'a>,
    },

    /// Phase 31: Field access: `p's x` or `the x of p`
    FieldAccess {
        object: &'a Expr<'a>,
        field: Symbol,
    },

    /// Phase 31: Constructor: `a new Point` or `a new Point with x 10 and y 20`
    /// Phase 34: Extended for generics: `a new Box of Int`
    New {
        type_name: Symbol,
        type_args: Vec<Symbol>,  // Empty for non-generic types
        init_fields: Vec<(Symbol, &'a Expr<'a>)>,  // Optional field initialization
    },

    /// Phase 33: Enum variant constructor: `a new Circle with radius 10`
    NewVariant {
        enum_name: Symbol,                      // Shape (resolved from registry)
        variant: Symbol,                        // Circle
        fields: Vec<(Symbol, &'a Expr<'a>)>,    // [(radius, 10)]
    },
}

/// Literal values in LOGOS.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Integer literal
    Number(i64),
    /// Text literal
    Text(Symbol),
    /// Boolean literal
    Boolean(bool),
    /// The nothing literal (unit type)
    Nothing,
    /// Character literal
    Char(char),
}

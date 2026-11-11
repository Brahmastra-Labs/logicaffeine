//! The **spatial IR** — the one program every spatial backend lowers FROM.
//!
//! A [`ir::SpatialProgram`] is a graph of *stages* (loads, stores, multicast receives, tile compute, scalar
//! dataflow bodies) connected through *bounded buffers* by typed *handshakes* (FIFO, semaphore pair,
//! barrier, valid/ready, token), placed onto a described mesh of *sites* ([`target::SpatialTarget`]) and
//! choreographed by a per-engine *schedule* of [`ir::ChoreoOp`]s. Its meaning is the executable token
//! semantics in [`eval`] (the golden oracle every lowering is proven against), and every verified program
//! carries a [`certificate::ChoreographyCertificate`] (deadlock-freedom + buffer bounds + per-PE SAT).
//!
//! The stakeholder's three words are three passes over this IR, never per-backend code: `size_buffers`
//! (buffering), `fuse_readers` (reader fusion / multicast), `synthesize_handshakes` (semaphore
//! choreography). A lowering reads only the IR and its target — the "choreography census" locks assert
//! that every semaphore / circular buffer / kernel a backend emits is the image of an IR object.
//!
//! A REWRITE of this IR — a pass, a legalization, an imported placement — is answerable to
//! [`equiv::equivalent_under`]: two programs, one corpus, a typed [`equiv::Divergence`] when they part
//! company. Structure is deliberately not consulted, because a legalization's whole job is to change it.

pub mod certificate;
pub mod clock;
pub mod compile_cache;
pub mod cost;
pub mod d5005;
pub mod decode;
pub mod decode_chip;
pub mod decode_engine;
pub mod describe;
pub mod drop;
pub mod equiv;
pub mod eval;
pub mod fit;
pub mod fpga_decode;
pub mod handoff;
pub mod int8_model;
pub mod ir;
pub mod lineage;
pub mod link;
pub mod lower;
pub mod part;
pub mod passes;
pub mod pipeline;
pub mod programs;
pub mod streamed;
pub mod rdu_chip;
pub mod residency;
pub mod roofs;
pub mod serving;
pub mod shim;
pub mod target;
pub mod tensor_parallel;
pub mod value_range;
pub mod wide_int;
pub mod work;

//! Stdlib-parity north-star corpus - domain: builder
//!
//! Stateful accumulator/cursor objects (builder.lg) - a StringBuilder and a ByteReader modelled as
//! LOGOS structs threaded functionally, byte-identical on every tier. Mirrors Go's strings.Builder,
//! bytes.Buffer, strings.Reader, bytes.Reader.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// A fresh empty builder. go: strings.Builder / bytes.Buffer
#[test]
fn parity_builder_new() {
    assert_all_tiers(
        "## Main\nShow \"[\" combined with builderString(newBuilder()) combined with \"]\".\n",
        "[]",
    );
}

/// Append a string. go: strings.WriteString / bytes.WriteString
#[test]
fn parity_builder_write() {
    assert_all_tiers(
        "## Main\nLet b be builderWriteString(builderWriteString(newBuilder(), \"Hello, \"), \"World\").\nShow builderString(b).\n",
        "Hello, World",
    );
}

/// Materialise the accumulated string.
#[test]
fn parity_builder_string() {
    assert_all_tiers(
        "## Main\nShow builderString(builderWriteString(newBuilder(), \"abc\")).\n",
        "abc",
    );
}

/// Length of the accumulated string.
#[test]
fn parity_builder_len() {
    assert_all_tiers("## Main\nShow builderLen(builderWriteString(builderWriteString(newBuilder(), \"ab\"), \"cde\")).\n", "5");
}

/// Reset the builder to empty. go: strings.Reset / bytes.Reset
#[test]
fn parity_builder_reset() {
    assert_all_tiers(
        "## Main\nLet b be builderWriteString(newBuilder(), \"x\").\nShow \"[\" combined with builderString(builderReset(b)) combined with \"]\".\n",
        "[]",
    );
}

/// Pre-grow capacity (a no-op hint). go: strings.Grow / bytes.Grow / slices.Grow
#[test]
fn parity_builder_grow() {
    assert_all_tiers(
        "## Main\nShow builderString(builderWriteString(builderGrow(newBuilder(), 16), \"hi\")).\n",
        "hi",
    );
}

/// A reader over a byte sequence. go: strings.NewReader / bytes.NewReader
#[test]
fn parity_reader_new() {
    assert_all_tiers(
        "## Main\nShow readerReadByte(newReader([65, 66, 67])).\n",
        "65",
    );
}

/// Read the byte at the cursor, then advance. go: strings.ReadByte / bytes.ReadByte
#[test]
fn parity_reader_read_byte() {
    assert_all_tiers(
        "## Main\nLet r be newReader([65, 66, 67]).\nShow readerReadByte(readerAdvance(r)).\nShow readerHasMore(readerAdvance(readerAdvance(readerAdvance(r)))).\n",
        "66\nfalse",
    );
}

/// Seek the cursor to an absolute position. go: strings.Seek / bytes.Seek
#[test]
fn parity_reader_seek() {
    assert_all_tiers(
        "## Main\nLet r be newReader([65, 66, 67]).\nShow readerReadByte(readerSeek(r, 3)).\n",
        "67",
    );
}

/// Append a single byte to the builder. go: strings.WriteByte / bytes.WriteByte
#[test]
fn parity_builder_write_byte() {
    assert_all_tiers(
        "## Main\nShow builderString(builderWriteByte(builderWriteString(newBuilder(), \"A\"), 66)).\n",
        "AB",
    );
}

/// Materialise the accumulated bytes as a sequence. go: bytes.Bytes
#[test]
fn parity_builder_bytes() {
    assert_all_tiers(
        "## Main\nShow builderBytes(builderWriteString(newBuilder(), \"AB\")).\n",
        "[65, 66]",
    );
}

/// Truncate the builder to the first n bytes. go: bytes.Truncate
#[test]
fn parity_builder_truncate() {
    assert_all_tiers(
        "## Main\nShow builderString(builderTruncate(builderWriteString(newBuilder(), \"Hello\"), 3)).\n",
        "Hel",
    );
}

/// Total length of the underlying sequence. go: bytes.Size / strings.Size
#[test]
fn parity_reader_size() {
    assert_all_tiers("## Main\nShow readerSize(newReader([65, 66, 67])).\n", "3");
}

/// Step the cursor back one byte. go: strings.UnreadByte / bytes.UnreadByte
#[test]
fn parity_reader_unread_byte() {
    assert_all_tiers(
        "## Main\nLet r be readerAdvance(readerAdvance(newReader([65, 66, 67]))).\nShow readerReadByte(readerUnreadByte(r)).\n",
        "66",
    );
}

/// Read the next n bytes from the cursor (no advance). go: bytes.Next / bytes.ReadBytes
#[test]
fn parity_reader_next() {
    assert_all_tiers(
        "## Main\nLet r be newReader([65, 66, 67]).\nShow readerNext(r, 2).\nShow readerNext(readerAdvance(r), 2).\n",
        "[65, 66]\n[66, 67]",
    );
}

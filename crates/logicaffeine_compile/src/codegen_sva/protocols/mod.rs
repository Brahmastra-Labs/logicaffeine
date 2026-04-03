//! Pre-Verified Protocol Templates
//!
//! Parameterizable SVA properties for standard hardware protocols.
//! Each template produces SvaProperty + English spec string.

pub mod riscv;

use super::sva_model::SvaExpr;
use super::SvaAssertionKind;

/// A protocol template — produces parameterized SVA properties.
#[derive(Debug, Clone)]
pub struct ProtocolProperty {
    pub name: String,
    pub spec: String,
    pub sva_body: String,
    pub kind: SvaAssertionKind,
}

/// AXI4 write channel handshake properties.
pub fn axi4_write_handshake(clock: &str) -> Vec<ProtocolProperty> {
    vec![
        ProtocolProperty {
            name: "AXI_AW_Handshake".into(),
            spec: "Always, if AWVALID is asserted, then eventually AWREADY responds.".into(),
            sva_body: "AWVALID |-> s_eventually(AWREADY)".into(),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "AXI_W_Follows_AW".into(),
            spec: "Always, if the address handshake completes, then eventually WVALID is asserted.".into(),
            sva_body: "(AWVALID && AWREADY) |-> s_eventually(WVALID)".into(),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "AXI_B_Follows_W".into(),
            spec: "Always, if the write data handshake completes, then eventually BVALID is asserted.".into(),
            sva_body: "(WVALID && WREADY) |-> s_eventually(BVALID)".into(),
            kind: SvaAssertionKind::Assert,
        },
    ]
}

/// APB setup/access phase properties.
pub fn apb_protocol(clock: &str) -> Vec<ProtocolProperty> {
    vec![
        ProtocolProperty {
            name: "APB_Setup_Phase".into(),
            spec: "Always, if PSEL is asserted without PENABLE, then PENABLE follows next cycle.".into(),
            sva_body: "(PSEL && !PENABLE) |=> PENABLE".into(),
            kind: SvaAssertionKind::Assert,
        },
        ProtocolProperty {
            name: "APB_Ready_In_Access".into(),
            spec: "Always, if PSEL and PENABLE are both asserted, eventually PREADY responds.".into(),
            sva_body: "(PSEL && PENABLE) |-> s_eventually(PREADY)".into(),
            kind: SvaAssertionKind::Assert,
        },
    ]
}

/// UART transmit properties.
pub fn uart_tx(clock: &str) -> Vec<ProtocolProperty> {
    vec![
        ProtocolProperty {
            name: "UART_TX_Busy".into(),
            spec: "Always, if tx_start is asserted, then tx_busy holds until transmission completes.".into(),
            sva_body: "tx_start |-> s_eventually(tx_busy)".into(),
            kind: SvaAssertionKind::Assert,
        },
    ]
}

/// SPI properties.
pub fn spi_protocol(clock: &str) -> Vec<ProtocolProperty> {
    vec![
        ProtocolProperty {
            name: "SPI_MOSI_Stable".into(),
            spec: "Always, when chip select is active, MOSI is stable on the clock edge.".into(),
            sva_body: "!ss |-> $stable(mosi)".into(),
            kind: SvaAssertionKind::Assert,
        },
    ]
}

/// I2C properties.
pub fn i2c_protocol(clock: &str) -> Vec<ProtocolProperty> {
    vec![
        ProtocolProperty {
            name: "I2C_Start_Condition".into(),
            spec: "A start condition is SDA falling while SCL is high.".into(),
            sva_body: "$fell(sda) && scl".into(),
            kind: SvaAssertionKind::Cover,
        },
        ProtocolProperty {
            name: "I2C_Stop_Condition".into(),
            spec: "A stop condition is SDA rising while SCL is high.".into(),
            sva_body: "$rose(sda) && scl".into(),
            kind: SvaAssertionKind::Cover,
        },
    ]
}

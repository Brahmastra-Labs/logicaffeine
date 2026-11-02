//! The digital control plane's frame schedule — DAC/ADC strobing around the analog plane.
//!
//! The mixed-signal division of labor: the DIGITAL side validates each word against its declared
//! range (the `Assume` contract), drives the DAC, waits out the settling budget, strobes the
//! sample-and-hold (the ZOH boundary), and reads the ADC. The analog side only ever settles.
//! A streamed reduction runs one FRAME per element with the accumulator resident in the S&H
//! between frames — the element stream is quantized per tick, the accumulator never is.

/// One step of the control plane's schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtrlStep {
    /// Drive DAC channel `ch` with the next validated word.
    LoadDac { ch: usize },
    /// Wait the target's DC settling budget.
    Settle,
    /// Strobe the sample-and-hold — commit this frame's result into analog state.
    SampleHold,
    /// Read ADC channel `ch` into a digital word.
    ReadAdc { ch: usize },
    /// Swap every tile's chopper polarity (the O5 scheduled transform): the SAME dataflow
    /// runs a second frame with input/output commutation reversed, so each frozen per-die
    /// offset enters negated and cancels EXACTLY in the two-frame average. What survives is
    /// intra-pair drift (`ChopResidual`, modeled) and noise (fresh per frame — averaging
    /// buys the √2).
    ChopSwap,
}

/// A frame-based control program: `frame` repeats `frames` times, then `epilogue` runs once.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlProgram {
    pub frame: Vec<CtrlStep>,
    pub frames: usize,
    pub epilogue: Vec<CtrlStep>,
    /// Wall-clock per frame in nanoseconds (the target's settling budget).
    pub tick_ns: f64,
}

impl ControlProgram {
    /// Total scheduled steps (frames × frame length + epilogue).
    pub fn total_steps(&self) -> usize {
        self.frames * self.frame.len() + self.epilogue.len()
    }

    /// The chopped twin of this schedule: every frame runs TWICE around a `ChopSwap`, and
    /// the digital side averages the pair. Purely a schedule transform — the dataflow, the
    /// netlist, and the certificates are untouched.
    pub fn chopped(&self) -> ControlProgram {
        let mut frame = self.frame.clone();
        frame.push(CtrlStep::ChopSwap);
        frame.extend(self.frame.iter().copied());
        frame.push(CtrlStep::ChopSwap);
        ControlProgram {
            frame,
            frames: self.frames,
            epilogue: self.epilogue.clone(),
            tick_ns: self.tick_ns,
        }
    }
}

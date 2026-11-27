use tetra_core::TdmaTime;

use tetra_pdus::phy::traits::rxtx_dev::{TxBurstPlacement, TxSlotBits};

use crate::phy::components::dsp_types::*;
use crate::phy::components::fir;
use crate::phy::components::modem_common::*;

/// Samples per symbol
const SPS: SampleCount = 4;

/// Samples per slot
pub const SAMPLES_SLOT: SampleCount = SPS * 255;

/// Output sample rate
pub const SAMPLE_RATE: f64 = 18000.0 * SPS as f64;

#[derive(PartialEq)]
pub enum Mode {
    /// Uplink modulation: a discontinuous burst (Normal or Control Uplink Burst)
    /// positioned within the slot by its burst delay, with silence (ramp/guard)
    /// outside the active part. Used by the MS. ETSI TS 100 392-2 cl. 9.4.3.4,
    /// Table 9.2.
    Ul,
}

pub struct Modulator {
    mode: Mode,
    /// Sample counter value at the beginning of hyperframe number 0
    reference_time: SampleCount,
    /// Pulse shaping filter
    filter: fir::FirComplexSym,
    dqpsk: DqpskMapper,
}

pub enum Error {
    /// Modulator needs data for another slot
    /// before it can continue producing TX signal.
    NeedMoreData,
}

impl Modulator {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            reference_time: 0,
            filter: fir::FirComplexSym::new(CHANNEL_FILTER_TAPS.len()),
            dqpsk: DqpskMapper::new(),
        }
    }

    /// Produce one output sample.
    pub fn sample(&mut self, sample_counter: SampleCount, tx_slot: &TxSlotBits) -> Result<ComplexSample, Error> {
        // Compensate for delay of pulse shaping filter in sample count
        let sample_counter = sample_counter + CHANNEL_FILTER_TAPS.len() as SampleCount;

        // Sample counter at beginning of current slot.
        // TODO: adjust self.reference_time when hyperframe number wraps to 0.
        // Now it breaks after 46 days.
        // This could also be further optimized by computing and storing it
        // only when a new slot becomes available.
        let slot_begin = self.reference_time + TdmaTime::to_int(tx_slot.time) as SampleCount * SAMPLES_SLOT;

        let mut sample = ComplexSample::ZERO;
        match self.mode {
            Mode::Ul => {
                // Uplink burst timing (cl. 9.4.3.4): the symbol time of SN(n) is
                // delayed by (n + d) symbol durations from the start of the slot.
                // SN0 (n = 0) is the differential phase reference and carries no
                // information. d is the burst delay from Table 9.2: 17 symbols for
                // both the Normal Uplink Burst (SNmax = 231) and the Control
                // Uplink Burst in subslot 1 (SNmax = 103); subslot 2 uses 144.5
                // symbols. Keep the half-symbol position in sample units.
                // Symbols outside the
                // active part are silence, so the shared antenna / PA is only
                // driven during the burst.
                let burst_delay = match tx_slot.placement {
                    TxBurstPlacement::FullSlot | TxBurstPlacement::Subslot1 => 17 * SPS,
                    TxBurstPlacement::Subslot2 => 17 * SPS + SAMPLES_SLOT / 2,
                };

                let sample_in_slot = sample_counter - slot_begin;
                let snmax = tx_slot.slot.map_or(0, |bits| (bits.len() / 2) as SampleCount);

                if sample_in_slot < burst_delay {
                    // Before SN0: silence (also lets the pulse-shaping filter
                    // settle before the active part).
                } else if sample_in_slot >= SAMPLES_SLOT {
                    // Slot is in the past; wait for the next burst.
                    return Err(Error::NeedMoreData);
                } else if let Some(bits) = tx_slot.slot
                    && (sample_in_slot - burst_delay) % SPS == 0
                {
                    // Symbol index relative to SN0.
                    let sn = (sample_in_slot - burst_delay) / SPS;
                    if sn == 0 {
                        // SN0: reset the differential reference and emit the
                        // reference symbol so SN1's phase transition is
                        // decodable at the BS.
                        self.dqpsk.reset_phase();
                        sample = self.dqpsk.reference();
                        // Fires exactly once per burst, at the start of the
                        // active part: definitive proof the uplink burst was
                        // actually synthesized (not silence), plus its timing
                        // alignment for tuning MS_TX_SAMPLE_DELAY.
                        tracing::debug!(
                            ts = %tx_slot.time,
                            slot_begin,
                            sample_counter,
                            snmax,
                            "Modulator: uplink burst active part emitted (SN0)"
                        );
                    } else if sn >= 1 && sn <= snmax {
                        let i = (sn - 1) as usize;
                        sample = self.dqpsk.symbol(bits[i * 2] != 0, bits[i * 2 + 1] != 0);
                    }
                    // sn > snmax: past the active part (guard), stay silent.
                }
            }
        }
        Ok(self.filter.sample(&CHANNEL_FILTER_TAPS, sample))
    }

    /// Align this modulator to the air-interface timing recovered by the
    /// downlink demodulator. `reference_time` is the sample-counter value at the
    /// beginning of hyperframe number 0 (the same definition the demodulator
    /// maintains), so `slot_begin` in [`Modulator::sample`] lands on the correct
    /// hardware sample position for an absolute TDMA slot time.
    ///
    /// An MS must call this every TX block because the
    /// demodulator continuously micro-adjusts its reference for sample slips.
    pub fn set_reference_time(&mut self, reference_time: SampleCount) {
        if self.mode == Mode::Ul {
            self.reference_time = reference_time;
        }
    }

    /// Diagnostic helper: the sample-counter position of the beginning of the
    /// slot for `tx_slot.time`, i.e. the `slot_begin` used in
    /// [`Modulator::sample`]. Only meaningful for [`Mode::Ul`] (returns `None`
    /// otherwise). Used to log where a pending uplink burst lands relative to
    /// the TX generation window so the RX->TX timing (`MS_TX_SAMPLE_DELAY`) can
    /// be diagnosed on hardware.
    pub fn ul_slot_begin(&self, tx_slot: &TxSlotBits) -> Option<SampleCount> {
        if self.mode == Mode::Ul {
            Some(self.reference_time + TdmaTime::to_int(tx_slot.time) as SampleCount * SAMPLES_SLOT)
        } else {
            None
        }
    }
}

struct DqpskMapper {
    pub phase: i8,
}

/// π/4-DQPSK constellation: maps an accumulated phase (in multiples of π/4) to a
/// constellation point. Generated in Python with:
///   import numpy as np
///   print(",\n".join("ComplexSample{ re: %9.6f, im: %9.6f }" % (v.real, v.imag)
///       for v in np.exp(1j*np.linspace(0, np.pi*2, 8, endpoint=False))))
// Preserve the original six-decimal table's f32 rounding, not FRAC_1_SQRT_2.
const LEGACY_DIAGONAL: f32 = f32::from_bits(0x3f35_04f7);
const CONSTELLATION: [ComplexSample; 8] = [
    ComplexSample {
        re: 1.000000,
        im: 0.000000,
    },
    ComplexSample {
        re: LEGACY_DIAGONAL,
        im: LEGACY_DIAGONAL,
    },
    ComplexSample {
        re: 0.000000,
        im: 1.000000,
    },
    ComplexSample {
        re: -LEGACY_DIAGONAL,
        im: LEGACY_DIAGONAL,
    },
    ComplexSample {
        re: -1.000000,
        im: 0.000000,
    },
    ComplexSample {
        re: -LEGACY_DIAGONAL,
        im: -LEGACY_DIAGONAL,
    },
    ComplexSample {
        re: -0.000000,
        im: -1.000000,
    },
    ComplexSample {
        re: LEGACY_DIAGONAL,
        im: -LEGACY_DIAGONAL,
    },
];

impl DqpskMapper {
    pub fn new() -> Self {
        Self { phase: 0 }
    }

    pub fn reset_phase(&mut self) {
        self.phase = 0;
    }

    /// Constellation point for the current phase without advancing it (SN0).
    pub fn reference(&self) -> ComplexSample {
        CONSTELLATION[self.phase as usize]
    }

    pub fn symbol(&mut self, bit0: bool, bit1: bool) -> ComplexSample {
        self.phase = (self.phase
            + match (bit0, bit1) {
                (true, true) => -3,
                (true, false) => -1,
                (false, false) => 1,
                (false, true) => 3,
            })
            & 7;
        CONSTELLATION[self.phase as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constellation_preserves_legacy_float_bits() {
        let expected = [
            (0x3f80_0000, 0),
            (0x3f35_04f7, 0x3f35_04f7),
            (0, 0x3f80_0000),
            (0xbf35_04f7, 0x3f35_04f7),
            (0xbf80_0000, 0),
            (0xbf35_04f7, 0xbf35_04f7),
            (0x8000_0000, 0xbf80_0000),
            (0x3f35_04f7, 0xbf35_04f7),
        ];
        for (sample, (re, im)) in CONSTELLATION.iter().zip(expected) {
            assert_eq!((sample.re.to_bits(), sample.im.to_bits()), (re, im));
        }
    }

    #[test]
    fn uplink_samples_match_independent_timing_and_pulse_synthesis() {
        for (placement, delay, nbits) in [
            (TxBurstPlacement::FullSlot, 68, 462),
            (TxBurstPlacement::Subslot1, 68, 206),
            (TxBurstPlacement::Subslot2, 578, 206),
        ] {
            let bits: Vec<u8> = (0..nbits).map(|i| ((i * 7 + i / 3) & 1) as u8).collect();
            let slot = TxSlotBits {
                time: TdmaTime::from_int(8),
                slot: Some(&bits),
                placement,
            };
            let mut modulator = Modulator::new(Mode::Ul);
            modulator.set_reference_time(321);
            let beginning = 321 + 8 * 1020;
            let mut impulses = [ComplexSample::ZERO; 1020];
            impulses[delay] = ComplexSample::new(1.0, 0.0);
            let mut phase: i32 = 0;
            for (i, dibit) in bits.chunks_exact(2).enumerate() {
                phase = (phase + [1, 3, -1, -3][(dibit[0] * 2 + dibit[1]) as usize]).rem_euclid(8);
                // Independent polar synthesis; legacy rounding is checked above.
                let angle = phase as f64 * std::f64::consts::FRAC_PI_4;
                let (im, re) = angle.sin_cos();
                let rounded = |v: f64| ((v * 1_000_000.0).round() / 1_000_000.0) as f32;
                impulses[delay + (i + 1) * 4] = ComplexSample::new(rounded(re), rounded(im));
            }
            for sample_index in 0..1020 {
                // Symmetric 32-tap FIR, written independently of FirComplexSym.
                let mut expected = ComplexSample::ZERO;
                for tap_index in 0..32.min(sample_index + 1) {
                    let tap = CHANNEL_FILTER_TAPS[if tap_index < 16 { 15 - tap_index } else { tap_index - 16 }];
                    expected += impulses[sample_index - tap_index] * tap;
                }
                let actual = modulator
                    .sample(beginning + sample_index as i64 - 16, &slot)
                    .unwrap_or_else(|_| panic!("slot ended early"));
                assert!(
                    (actual - expected).norm() < 1e-6,
                    "{placement:?} sample {sample_index}: {actual:?} != {expected:?}"
                );
            }
            assert!(matches!(modulator.sample(beginning + 1020 - 16, &slot), Err(Error::NeedMoreData)));
        }
    }
}

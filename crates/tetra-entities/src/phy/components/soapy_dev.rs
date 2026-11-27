//! Resampling, buffering and timestamp handling
//! between SDR device and modulator/demodulator code.

use rustfft;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tetra_config::magentastation::SharedConfig;
use tetra_core::TdmaTime;

use tetra_pdus::phy::traits::rxtx_dev::MsTxLookahead;
use tetra_pdus::phy::traits::rxtx_dev::RfPath;
use tetra_pdus::phy::traits::rxtx_dev::RxSlotBits;
use tetra_pdus::phy::traits::rxtx_dev::RxTxDev;
use tetra_pdus::phy::traits::rxtx_dev::RxTxDevError;
use tetra_pdus::phy::traits::rxtx_dev::TxSlotBits;

use super::demodulator;
use super::dsp_types::*;
use super::fcfb;
use super::modulator;
use super::soapyio;

/// Fixed sample offset added to the downlink-derived timing reference when
/// aligning the MS uplink modulator. It absorbs the residual delay between the
/// RX and TX signal chains (analog front end + DSP pipeline) so the burst lands
/// within the uplink burst timing / guard period (ETSI TS 100 392-2 cl. 9.4.3.4;
/// note TETRA has no active timing-advance procedure -- MS-BS propagation is
/// absorbed by the uplink guard period). Zero is the nominal starting point;
/// this is the single knob to tune against a real base station if the burst
/// lands slightly early/late in the uplink slot. Positive = transmit later.
const MS_TX_SAMPLE_DELAY: SampleCount = 0;

/// While the MS downlink is unsynchronized the demodulator stays in
/// `Mode::DlUnsynchronized` and never yields a demodulated slot, so
/// `rxtx_timeslot`'s RX loop would run continuously until the MS camps on a
/// cell. Because the stack is single-threaded and receive-timed, that would
/// starve the run loop of any chance to make progress before synchronization —
/// in particular it could not service the external control interface (config
/// read/staging must work as soon as the control link is up, before the MS has
/// synced or registered) nor drive scan-dwell cell selection (cl. 18.3.4).
///
/// To keep the run loop responsive, the unsynchronized RX loop yields back to
/// the caller (returning with no demodulated slot) after this much wall-clock
/// time. The demodulator's correlation state persists across calls, so
/// re-acquisition simply resumes on the next invocation. This affects only the
/// unsynchronized state: once the downlink is synchronized the loop returns each
/// slot on its own.
const UNSYNC_YIELD: Duration = Duration::from_millis(20);

pub struct SdrConfig<'a> {
    /// SoapySDR device arguments
    pub dev_args: &'a [(&'a str, &'a str)],
    /// SDR RX center frequency
    pub rx_freq: Option<f64>,
    /// SDR TX center frequency
    pub tx_freq: Option<f64>,
}

#[derive(Default)]
pub struct PhyConfig<'a> {
    /// Downlink carrier frequencies to receive.
    pub downlink_frequencies: &'a [f64],
    /// Uplink carrier frequencies for an MS to transmit on. Modulated with the
    /// uplink burst timing (discontinuous bursts, ETSI TS 100 392-2 cl. 9.4.3.4).
    pub ms_ul_frequencies: &'a [f64],
}

pub struct RxTxDevSoapySdr {
    sdr: soapyio::SoapyIo,
    rx_dsp: Option<RxDsp>,
    tx_dsp: Option<TxDsp>,
    /// Cooperative shutdown flag (MS mode). While the MS is unsynchronized the
    /// downlink demodulator stays in `Mode::DlUnsynchronized` and never yields a
    /// demodulated slot, so `rxtx_timeslot`'s RX loop would otherwise spin until
    /// the MS camps on a cell — meaning a Ctrl+C while not connected to a base
    /// station is only acted on once a downlink appears. When this flag is
    /// present and cleared, the RX loop returns promptly so the run loop can
    /// observe the stop request and shut down.
    run_flag: Option<Arc<AtomicBool>>,
}

type FftPlanner = rustfft::FftPlanner<RealSample>;

impl RxTxDevSoapySdr {
    pub fn new(cfg: &SharedConfig) -> Self {
        let mut fft_planner = rustfft::FftPlanner::new();

        let config_guard = cfg.config();
        let soapy_cfg = config_guard
            .as_ref()
            .phy_io
            .soapysdr
            .as_ref()
            .expect("Soapysdr config must be set for Soapysdr PhyIo");

        let (dl_corrected, dl_err) = soapy_cfg.dl_freq_corrected();
        let (ul_corrected, ul_err) = soapy_cfg.ul_freq_corrected();

        tracing::info!(
            "Freqs: DL / UL: {:.6} MHz / {:.6} MHz   PPM: {:.2} -> err {:.0} / {:.0} hz, adj {:.6} MHz / {:.6} MHz",
            soapy_cfg.dl_freq / 1e6,
            soapy_cfg.ul_freq / 1e6,
            soapy_cfg.ppm_err,
            dl_err,
            ul_err,
            dl_corrected / 1e6,
            ul_corrected / 1e6
        );

        // Frequency bindings are held here so the PhyConfig slices stay valid
        // for the lifetime of the RxDsp/TxDsp construction below.
        let dl_freqs = [dl_corrected];
        // Keep the modulator at the provisional TX centre until SYSINFO supplies
        // the uplink carrier. PhyMs refuses bursts before that retune.
        let ms_ul_freqs = [if soapy_cfg.ul_freq > 0.0 { ul_corrected } else { dl_corrected }];
        let phy_config = PhyConfig {
            downlink_frequencies: &dl_freqs,
            ms_ul_frequencies: &ms_ul_freqs,
        };

        let mut sdr = soapyio::SoapyIo::new(cfg).unwrap();

        Self {
            rx_dsp: if sdr.rx_enabled() {
                Some(RxDsp::new(&mut fft_planner, &mut sdr, &phy_config))
            } else {
                None
            },

            tx_dsp: if sdr.tx_enabled() && !phy_config.ms_ul_frequencies.is_empty() {
                Some(TxDsp::new(&mut fft_planner, &mut sdr, &phy_config))
            } else {
                None
            },

            sdr,
            run_flag: None,
        }
    }

    /// Install the cooperative shutdown flag (MS mode). Passed the process-wide
    /// `is_running` flag so the blocking RX loop can bail out promptly when a
    /// shutdown is requested even while the MS is not yet synchronized to a base
    /// station.
    pub fn set_run_flag(&mut self, flag: Arc<AtomicBool>) {
        self.run_flag = Some(flag);
    }

    /// True when a cooperative shutdown has been requested (run flag present and
    /// cleared). False when no flag is installed.
    fn stop_requested(&self) -> bool {
        self.run_flag.as_ref().is_some_and(|f| !f.load(Ordering::Relaxed))
    }

    /// True when the serving-cell downlink is synchronized. `false` when there
    /// is no RX DSP (TX-only) or no downlink lock yet.
    fn dl_synchronized(&self) -> bool {
        self.rx_dsp.as_ref().is_some_and(|rx| rx.dl_synchronized())
    }

    /// Process a block of received signal.
    /// Return true if processing can be continued,
    /// false if a slot has been demodulated and rxtx_timeslot should return.
    fn process_rx_block(&mut self) -> Result<bool, RxTxDevError> {
        if let Some(rx_dsp) = &mut self.rx_dsp {
            rx_dsp.process_block(&mut self.sdr)
        } else {
            Ok(false)
        }
    }

    /// Produce a block of transmit signal.
    /// Return true if processing can be continued,
    /// false if more data is needed
    /// or if it wants to wait before producing more.
    fn process_tx_block(&mut self, tx_slot: &[TxSlotBits]) -> Result<bool, RxTxDevError> {
        // Read the current downlink timing reference (and RX block position)
        // from the RX DSP first, so the immutable borrows end before we take a
        // mutable borrow of the TX DSP. `dl_reference` aligns the MS uplink
        // modulator to the recovered air-interface clock (see TxDsp::process_block).
        let rx_block_count = self.rx_dsp.as_ref().map(|rx_dsp| rx_dsp.rx_block_count);
        let dl_reference = self.rx_dsp.as_ref().and_then(|rx_dsp| rx_dsp.dl_reference_time());
        if let Some(tx_dsp) = &mut self.tx_dsp {
            if self.sdr.tx_possible() {
                tx_dsp.process_block(&mut self.sdr, rx_block_count, dl_reference, tx_slot)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }
}

impl RxTxDev for RxTxDevSoapySdr {
    fn rxtx_timeslot<'a>(
        &'a mut self,
        tx_slot: &[TxSlotBits],
        // TODO multiple demodulators
    ) -> Result<Vec<Option<RxSlotBits<'a>>>, RxTxDevError> {
        // First generate as much TX signal as possible at the moment.
        while self.process_tx_block(tx_slot)? {}

        // Reference for the unsynchronized cooperative-yield window (see
        // UNSYNC_YIELD). Cheap timestamp read; ignored while synchronized.
        let unsync_start = Instant::now();

        while self.process_rx_block()? {
            // While the MS is unsynchronized the downlink demodulator
            // (Mode::DlUnsynchronized) never yields a demodulated slot, so this
            // loop would otherwise spin until the MS camps on a cell. Because
            // the stack is single-threaded and receive-timed, that starves the
            // run loop before synchronization. Restricted to the unsynchronized
            // case so a synchronized/camped MS is untouched: there the loop
            // already returns each slot on its own (letting MM transmit e.g. its
            // U-ITSI DETACH before the streams close).
            if !self.dl_synchronized() {
                // Cooperative shutdown: a Ctrl+C while not connected to a base
                // station is observed here so the run loop can exit promptly
                // instead of only once a downlink appears.
                if self.stop_requested() {
                    break;
                }
                // Cooperative yield: return with no slot so the run loop regains
                // control periodically and can service the external control
                // interface (offline config/state) and drive scan-dwell cell
                // selection even though the downlink is not yet synchronized.
                if unsync_start.elapsed() >= UNSYNC_YIELD {
                    break;
                }
            }
            // Continue producing TX signal if possible.
            while self.process_tx_block(tx_slot)? {}
        }

        if let Some(rx_dsp) = &mut self.rx_dsp {
            Ok(rx_dsp.take_slot_bits())
        } else {
            Ok(Default::default())
        }
    }

    fn set_rf_path(&mut self, path: RfPath) {
        // Current hardware is full duplex (separate RX and TX chains), so the
        // antenna does not need to be switched and this is intentionally a
        // no-op beyond a trace.
        //
        // FUTURE (half-duplex front end): drive the antenna T/R changeover here,
        // e.g. toggle a GPIO / RF-switch line so the shared antenna is connected
        // to the PA for RfPath::Tx and to the LNA for RfPath::Rx. The SoapySDR
        // GPIO API (`self.sdr` -> `Device::write_gpio`) or a board-specific
        // control line is the intended extension point. Keep the switch settling
        // time within the uplink guard period (ETSI TS 100 392-2 cl. 9.4.3.4).
        tracing::trace!(?path, "set_rf_path (full-duplex: no antenna switching)");
    }

    fn tx_air_time(&self) -> Option<TdmaTime> {
        // Local-basis TDMA slot at the TX *generation frontier* — the point the
        // modulator is currently producing signal for. This is the line the MS
        // uplink scheduler must stay ahead of.
        //
        // It is deliberately the generation frontier (`block_count`), not the
        // true hardware "now": block generation runs a look-ahead window
        // (`dmin`..`dmax`) ahead of real time, and the modulator only emits a
        // burst while it sweeps forward through the slot. A burst scheduled at
        // true-now + a couple slots can therefore still land *behind* the
        // already-advanced generation pointer and be produced as silence. Using
        // the frontier makes the lead self-calibrating against that look-ahead.
        //
        // `reference_time` is the modem-sample position of local slot 0 (from
        // the downlink demodulator); the frontier is in the same modem-sample
        // domain, so their difference divided by the slot length is the slot
        // index. `None` until downlink lock, before TX is possible, or before
        // any block has been generated.
        let reference_time = self.rx_dsp.as_ref()?.dl_reference_time()?;
        if !self.sdr.tx_possible() {
            return None;
        }
        let frontier = self.tx_dsp.as_ref()?.generation_frontier()?;
        let slot = (frontier - reference_time).div_euclid(modulator::SAMPLES_SLOT) as i32;
        Some(TdmaTime::from_int(slot))
    }

    fn ms_tx_lookahead(&self) -> Option<MsTxLookahead> {
        self.tx_dsp.as_ref()?.lookahead(&self.sdr)
    }

    fn dl_rssi_dbfs(&self) -> Option<f32> {
        self.rx_dsp.as_ref()?.dl_rssi_dbfs()
    }

    fn set_rx_frequency(&mut self, carrier_hz: f64) {
        if let Err(e) = self.sdr.set_rx_frequency(carrier_hz) {
            tracing::error!("SoapySDR: failed to retune RX to {carrier_hz} Hz: {e}");
            return;
        }
        // Drop the previous carrier's recovered downlink timing so the MS
        // re-acquires promptly on the new carrier (scanning, cl. 18.3.4).
        if let Some(rx_dsp) = self.rx_dsp.as_mut() {
            rx_dsp.reset_dl_demodulators();
        }
    }

    fn set_tx_frequency(&mut self, carrier_hz: f64) -> Result<(), RxTxDevError> {
        // Move the TX LO only. The uplink modulator was built at the provisional
        // TX centre (digital offset 0), so the burst lands on `carrier_hz` with
        // no channelizer/modulator rebuild — symmetric with set_rx_frequency but
        // without the RX intermediate-frequency offset. Used by the camp-time
        // uplink derivation once the cell's duplex parameters are known
        // (EN 300 392-2 cl. 18.4.2.2).
        self.sdr.set_tx_frequency(carrier_hz).map_err(|e| {
            tracing::error!("SoapySDR: failed to retune TX to {carrier_hz} Hz: {e}");
            RxTxDevError::TxTuneError
        })
    }
}

struct RxDsp {
    rx_fcfb: fcfb::AnalysisInputProcessor,

    rx_block_size: fcfb::InputBlockSize,
    rx_buffer: Vec<ComplexSample>,
    /// How much of rx_buffer has been filled
    rx_buffer_i: usize,
    rx_block_count: fcfb::BlockCount,

    downlinks: Vec<DemodulatorChannel>,
}

impl RxDsp {
    fn new(fft_planner: &mut FftPlanner, sdr: &mut soapyio::SoapyIo, phy_config: &PhyConfig) -> Self {
        let sdr_sample_rate = sdr.rx_sample_rate();
        let rx_fcfb_params = fcfb::AnalysisInputParameters {
            // Use a bin spacing of 500 Hz.
            // This is a submultiple of the 72 kHz modem sample rate
            // and allows tuning in steps of 500 Hz.
            fft_size: (sdr_sample_rate / 500.0).round() as usize,
            center_frequency: sdr.rx_center_frequency().unwrap(),
            sample_rate: sdr_sample_rate,
            overlap: fcfb::Overlap::O1_4,
        };

        let fcfb = fcfb::AnalysisInputProcessor::new(fft_planner, rx_fcfb_params);
        let rx_block_size = fcfb.input_block_size();

        Self {
            rx_block_size,
            rx_buffer: vec![num::zero(); rx_block_size.overlap + rx_block_size.new],
            rx_buffer_i: 0,
            rx_fcfb: fcfb,
            rx_block_count: 0,

            downlinks: phy_config
                .downlink_frequencies
                .iter()
                .map(|dl_freq| DemodulatorChannel::new(fft_planner, rx_fcfb_params, *dl_freq, demodulator::Mode::DlUnsynchronized))
                .collect(),
        }
    }

    /// Sample-counter reference (beginning of hyperframe number 0) of the first
    /// synchronized downlink demodulator. Used to align the MS uplink transmit
    /// modulator to the recovered air-interface timing. `None` until downlink
    /// lock is achieved (before that, nothing is transmitted anyway).
    fn dl_reference_time(&self) -> Option<SampleCount> {
        self.downlinks.iter().find_map(|dl| dl.demodulator.synchronized_reference_time())
    }

    /// True when any serving-cell downlink demodulator has achieved downlink
    /// lock. While this is false the demodulators never yield a slot, so the
    /// RX processing loop only terminates once a shutdown is requested.
    fn dl_synchronized(&self) -> bool {
        self.downlinks.iter().any(|dl| dl.is_synchronized())
    }

    /// Reset every serving-cell downlink demodulator to `DlUnsynchronized`,
    /// discarding the previous carrier's recovered timing. Called on a downlink
    /// retune (MS scanning cell selection, cl. 18.3.4) so re-acquisition starts
    /// immediately on the new carrier instead of after the demodulator's own
    /// ~100-slot sync-loss revert.
    fn reset_dl_demodulators(&mut self) {
        for dl in self.downlinks.iter_mut() {
            dl.demodulator.reset(demodulator::Mode::DlUnsynchronized);
        }
    }

    /// Most recent uncalibrated downlink RSSI (dBFS) from the serving-cell
    /// downlink demodulator (the first monitor pair's downlink channel). `None`
    /// until a downlink slot has been demodulated. Measured relative to the
    /// demodulator full-scale magnitude, so it tracks receive level for the MLE
    /// reselection input (cl. 18.3.4) and the management UI, not absolute power.
    fn dl_rssi_dbfs(&self) -> Option<f32> {
        self.downlinks.iter().find_map(|dl| dl.demodulator.dl_rssi_dbfs())
    }

    fn process_block(&mut self, sdr: &mut soapyio::SoapyIo) -> Result<bool, RxTxDevError> {
        self.receive_block(sdr)?;
        let fcfb_result = self.rx_fcfb.process(&self.rx_buffer[..], self.rx_block_count);

        let mut continue_processing = true;

        for demod in self.downlinks.iter_mut() {
            continue_processing = demod.process(fcfb_result, self.rx_block_count) && continue_processing;
        }

        Ok(continue_processing)
    }

    fn receive_block(&mut self, sdr: &mut soapyio::SoapyIo) -> Result<(), RxTxDevError> {
        self.receive_block_with(|buffer| sdr.receive(buffer))
    }

    fn receive_block_with(
        &mut self,
        mut receive: impl FnMut(&mut [ComplexSample]) -> Result<soapyio::RxResult, RxTxDevError>,
    ) -> Result<(), RxTxDevError> {
        self.rx_block_count += 1;

        // Copy overlapping part from previous block to the beginning
        self.rx_buffer
            .copy_within(self.rx_block_size.new..self.rx_block_size.new + self.rx_block_size.overlap, 0);
        self.rx_buffer_i = self.rx_block_size.overlap;

        loop {
            let available = self.rx_buffer.len() - self.rx_buffer_i;
            let result = receive(&mut self.rx_buffer[self.rx_buffer_i..])?;
            if result.len == 0 || result.len > available {
                return Err(RxTxDevError::RxReadError);
            }

            let block_size = self.rx_block_size.new as SampleCount;
            let expected_count = self.rx_block_count as SampleCount * block_size + self.rx_buffer_i as SampleCount;
            let samples_lost = result.count - expected_count;
            if samples_lost != 0 {
                // Samples have been lost.
                // Mark RX buffer as empty and skip the right number of samples
                // to receive the next full processing block in the next iteration.

                // Expected sample count for the next read,
                // assuming no more samples are lost.
                let next_count = result.count + result.len as SampleCount;
                // div_euclid always rounds down (towards negative numbers),
                // so use it with negations to round up to the next block.
                let next_possible_block = -(-next_count).div_euclid(block_size) + 1;
                let next_block_beginning = next_possible_block * block_size;

                let mut samples_to_skip = next_block_beginning - next_count;
                let mut expected_count = next_count;

                tracing::warn!(
                    "Lost {} samples, skipping {} more samples and {} processing blocks",
                    samples_lost,
                    samples_to_skip,
                    next_possible_block - self.rx_block_count
                );

                self.rx_block_count = next_possible_block;
                self.rx_buffer_i = 0;

                // Repeat reads until the correct number of samples has been skipped.
                while samples_to_skip > 0 {
                    let request = samples_to_skip.min(self.rx_buffer.len() as SampleCount) as usize;
                    let result = receive(&mut self.rx_buffer[..request])?;
                    if result.len == 0 || result.len > request {
                        return Err(RxTxDevError::RxReadError);
                    }
                    let next_count = result.count + result.len as SampleCount;
                    if result.count != expected_count {
                        // Discard reads may themselves cross another gap. Use
                        // their timestamp, not only their length, to realign.
                        self.rx_block_count = -(-next_count).div_euclid(block_size) + 1;
                    }
                    expected_count = next_count;
                    samples_to_skip = self.rx_block_count * block_size - next_count;
                }
            } else {
                self.rx_buffer_i += result.len;
                if self.rx_buffer_i == self.rx_buffer.len() {
                    // tracing::trace!("Received processing block {} ({} samples in SDR buffer)",
                    //     self.rx_block_count,
                    //     // incorrect if time is not available but does not really matter
                    //     sdr.rx_current_count().unwrap_or(0) - (result.count + result.len as SampleCount - 1),
                    // );
                    return Ok(());
                }
            }
        }
    }

    fn take_slot_bits<'a>(&'a mut self) -> Vec<Option<RxSlotBits<'a>>> {
        // TODO: avoid dynamic allocation here?
        let mut slot_bits = Vec::with_capacity(self.downlinks.len());
        for demod in self.downlinks.iter_mut() {
            slot_bits.push(demod.demodulator.take_demodulated_slot());
        }

        slot_bits
    }
}

struct TxDsp {
    fcfb: fcfb::SynthesisOutputProcessor,
    block_count: fcfb::BlockCount,
    modulators: Vec<ModulatorChannel>,
}

impl TxDsp {
    fn new(fft_planner: &mut FftPlanner, sdr: &mut soapyio::SoapyIo, phy_config: &PhyConfig) -> Self {
        let sdr_sample_rate = sdr.tx_sample_rate();
        let fcfb_params = fcfb::SynthesisOutputParameters {
            ifft_size: (sdr_sample_rate / 500.0).round() as usize,
            center_frequency: sdr.tx_center_frequency().unwrap(),
            sample_rate: sdr_sample_rate,
            overlap: fcfb::Overlap::O1_4,
        };

        let fcfb = fcfb::SynthesisOutputProcessor::new(fft_planner, fcfb_params);

        let mut modulators = Vec::<ModulatorChannel>::new();
        for ul_freq in phy_config.ms_ul_frequencies {
            modulators.push(ModulatorChannel::new(fft_planner, fcfb_params, *ul_freq, modulator::Mode::Ul));
        }

        Self {
            fcfb,
            block_count: 0,
            modulators,
        }
    }

    fn process_block(
        &mut self,
        sdr: &mut soapyio::SoapyIo,
        latest_rx_block: Option<fcfb::BlockCount>,
        dl_reference: Option<SampleCount>,
        tx_slot: &[TxSlotBits],
    ) -> Result<bool, RxTxDevError> {
        // MS uplink: align the uplink modulator(s) to the downlink timing
        // recovered by the RX demodulator, offset by the fixed RX->TX chain
        // delay (see MS_TX_SAMPLE_DELAY). Without this the uplink modulator's timing
        // reference stays at 0 while the burst time carries the base station's
        // absolute (large) TDMA clock, so the burst is scheduled at an
        // impossible far-future sample position and only silence is ever
        // transmitted. Re-applied every block because the demodulator
        // continuously micro-adjusts its reference.
        if let Some(reference_time) = dl_reference {
            let aligned = reference_time + MS_TX_SAMPLE_DELAY;
            for modulator in self.modulators.iter_mut() {
                modulator.set_reference_time(aligned);
            }
        }

        // Discontinuous MS uplink: emit nothing when there is no burst to send
        // this cycle. Streaming idle silence would force the generation frontier
        // (`block_count`) to run the full `dmax` look-ahead ahead of the DAC on
        // every cycle, which is exactly what pushes a BS-granted uplink slot
        // (`dltime + 2`, cl. 9.3.9) behind the frontier and makes it
        // unreachable. By not writing idle blocks the frontier collapses back
        // toward the DAC (re-anchored via the `dmin` skip below on the next
        // burst), so the granted slot stays ahead of the frontier and the burst
        // is produced in it. The SoapySX driver fills every un-written sample
        // with silence and keys the PA off for it, so skipping generation is
        // exactly equivalent to transmitting silence — minus the frontier lead.
        let have_burst = tx_slot.iter().any(|slot| slot.slot.is_some());
        if !have_burst {
            return Ok(false);
        }

        let current_sample = sdr.tx_current_count()?;
        // Current time as block count
        let current_block = current_sample.div_euclid(self.fcfb.output_block_size() as SampleCount);

        let d = self.block_count - current_block;
        // Skip TX blocks in the past or in too near future
        let dmin = 2; // how many blocks in future minimum
        if d < dmin {
            let new_block_count = current_block + dmin;
            let skipped = new_block_count - self.block_count;
            // For a discontinuous (MS) uplink this fast-forward is the *expected*
            // re-anchor, not an underrun: between bursts the transmitter is
            // silent (see the idle-skip above), so the frontier collapses back
            // toward the DAC and the next burst pulls block_count forward to
            // just ahead of it. That is exactly what keeps the granted uplink
            // slot (dltime+2, cl. 9.3.9) reachable.
            tracing::debug!(
                "Discontinuous TX re-anchor: skipping {} TX blocks to block {}",
                skipped,
                new_block_count
            );
            self.block_count = new_block_count;
        }
        // Limit how far into future TX blocks are generated.
        let dmax = Self::TX_GEN_MAX_BLOCKS;
        if d > dmax {
            return Ok(false);
        }
        // Also limit how far from the latest RX block TX blocks are generated.
        // This prevents TX from ending up in an infinite loop
        // which does not give a chance for RX signal to get processed.
        //
        // This is not strictly necessary right now but might become useful
        // with different modulator operating modes in the future.
        //
        // Maybe the limit using hardware time above is redundant.
        if let Some(latest_rx_block) = latest_rx_block {
            let d_rx = self.block_count - latest_rx_block;
            if d_rx > dmax {
                return Ok(false);
            }
        }

        for (modulator, tx_slot) in self.modulators.iter_mut().zip(tx_slot) {
            if !modulator.process(&mut self.fcfb, self.block_count, tx_slot) {
                return Ok(false);
            }
        }

        let tx_signal = self.fcfb.process();

        // TODO: compensate for delay of SDR
        let sdr_sample_count = tx_signal.len() as SampleCount * self.block_count;

        // Increment block count before calling sdr.transmit with ?,
        // so we do not end up producing the same block again even if transmit fails.
        self.block_count += 1;

        sdr.transmit(tx_signal, Some(sdr_sample_count))?;

        // tracing::trace!("Produced transmit block {} ({} samples in future)",
        //     self.block_count - 1,
        //     sdr_sample_count - sdr.tx_current_count().unwrap_or(0),
        // );

        Ok(true)
    }

    /// Modem-sample position of the current TX generation frontier — the next
    /// block the modulators will produce (`block_first = block_count *
    /// modem_block_len`, the same quantity the modulator times bursts against).
    ///
    /// This leads the true hardware "now" by the TX look-ahead window (the
    /// `dmin`..`dmax` block gating in [`Self::process_block`]). Uplink bursts
    /// must be scheduled *ahead of this frontier*, not merely ahead of true
    /// time: block generation only advances, so a slot whose start already sits
    /// behind the frontier is produced as silence and never revisited. `None`
    /// before any modulator exists.
    fn generation_frontier(&self) -> Option<SampleCount> {
        let modem_block_len = self.modulators.first()?.modem_block_len() as SampleCount;
        Some(self.block_count * modem_block_len)
    }

    /// Maximum number of modem TX blocks the generator produces ahead of the
    /// hardware DAC pointer (the look-ahead cap enforced in
    /// [`Self::process_block`]). A larger window guards against DAC underruns
    /// but pushes the generation frontier further ahead of real time, which is
    /// what makes a BS-granted reserved uplink slot unreachable (see
    /// [`MsTxLookahead`] / `PhyMs::schedule_uplink_time`).
    const TX_GEN_MAX_BLOCKS: fcfb::BlockCount = 60;

    /// Measure the current TX look-ahead: how far the generation frontier leads
    /// the hardware DAC pointer, in modem blocks and in timeslots. This is the
    /// **T** term of the reserved-slot reachability budget (see
    /// [`MsTxLookahead`]); it is a pure diagnostic and changes nothing.
    ///
    /// `blocks = block_count - DAC_block` is computed exactly as the look-ahead
    /// gate in [`Self::process_block`] (`d`), so it is directly comparable to
    /// [`Self::TX_GEN_MAX_BLOCKS`]. Each block spans `modem_block_len` modem
    /// samples, and a timeslot is `SAMPLES_SLOT` modem samples, so the block
    /// count converts to timeslots independently of the SDR sample rate.
    fn lookahead(&self, sdr: &soapyio::SoapyIo) -> Option<MsTxLookahead> {
        if !sdr.tx_possible() {
            return None;
        }
        let current_sample = sdr.tx_current_count().ok()?;
        let output_block_size = self.fcfb.output_block_size() as SampleCount;
        if output_block_size == 0 {
            return None;
        }
        let current_block = current_sample.div_euclid(output_block_size);
        let blocks = self.block_count - current_block;
        let modem_block_len = self.modulators.first()?.modem_block_len() as f64;
        Some(MsTxLookahead {
            blocks,
            max_blocks: Self::TX_GEN_MAX_BLOCKS,
            slots: blocks as f64 * modem_block_len / modulator::SAMPLES_SLOT as f64,
        })
    }
}

struct DemodulatorChannel {
    downconverter: fcfb::AnalysisOutputProcessor,
    demodulator: demodulator::Demodulator,
}

impl DemodulatorChannel {
    fn new(
        fft_planner: &mut FftPlanner,
        analysis_in_params: fcfb::AnalysisInputParameters,
        frequency: f64,
        mode: demodulator::Mode,
    ) -> Self {
        Self {
            downconverter: fcfb::AnalysisOutputProcessor::new_with_frequency(
                fft_planner,
                analysis_in_params,
                demodulator::SAMPLE_RATE,
                frequency,
                Some(25000.0),
            ),
            demodulator: demodulator::Demodulator::new(mode),
        }
    }

    /// Return true if processing should be continued,
    /// false if a new demodulated slot is available.
    fn process(&mut self, fcfb_result: &fcfb::AnalysisIntermediateResult, block_count: fcfb::BlockCount) -> bool {
        let samples = self.downconverter.process(fcfb_result);
        for (i, sample) in samples.iter().enumerate() {
            // TODO: include delay of FCFB in sample count
            self.demodulator.sample(
                *sample,
                block_count as SampleCount * samples.len() as SampleCount + i as SampleCount,
            );
        }
        !self.demodulator.demodulated_slot_available()
    }

    /// True once this channel's demodulator has downlink lock (`Mode::Dl`).
    fn is_synchronized(&self) -> bool {
        self.demodulator.is_synchronized()
    }
}

struct ModulatorChannel {
    upconverter: fcfb::SynthesisInputProcessor,
    modulator: modulator::Modulator,
    /// Buffer for modulated signal at modulator sample rate.
    buffer: fcfb::InputBuffer,
    /// How much of buffer is filled
    buffer_i: usize,
    /// Last uplink burst time for which a placement diagnostic was logged, so
    /// the per-block diagnostic fires at most once per queued burst.
    diag_last_time: Option<TdmaTime>,
}

impl ModulatorChannel {
    fn new(
        fft_planner: &mut FftPlanner,
        synthesis_out_params: fcfb::SynthesisOutputParameters,
        frequency: f64,
        mode: modulator::Mode,
    ) -> Self {
        let upconverter = fcfb::SynthesisInputProcessor::new_with_frequency(
            fft_planner,
            synthesis_out_params,
            modulator::SAMPLE_RATE,
            frequency,
            Some(25000.0),
        );
        Self {
            buffer: upconverter.make_input_buffer(),
            buffer_i: 0,
            upconverter,
            modulator: modulator::Modulator::new(mode),
            diag_last_time: None,
        }
    }

    fn set_reference_time(&mut self, reference_time: SampleCount) {
        self.modulator.set_reference_time(reference_time);
    }

    /// Number of new modem-rate samples the modulator produces per TX block.
    /// This is the multiplier between the TX block count and the modem sample
    /// counter (`block_first = block_count * modem_block_len`), so it converts
    /// the generation frontier into the sample domain the burst timing uses.
    fn modem_block_len(&self) -> usize {
        self.upconverter.input_block_size().new
    }

    fn process(&mut self, fcfb: &mut fcfb::SynthesisOutputProcessor, block_count: fcfb::BlockCount, tx_slot: &TxSlotBits) -> bool {
        let buf = self.buffer.buffer_in();

        // Diagnostic: when an uplink burst is queued, log once per burst where
        // its active part lands relative to the TX generation window. The
        // modulator only emits when the TX sample counter sweeps through
        // `slot_begin`; if `slot_begin` is already behind `block_first` when the
        // block is generated the burst is perpetually "in the past" and only
        // silence is transmitted (SN0 never fires). `ahead_samples` shows the
        // sign/magnitude of the misalignment for tuning `MS_TX_SAMPLE_DELAY`.
        if tx_slot.slot.is_some()
            && self.diag_last_time != Some(tx_slot.time)
            && let Some(slot_begin) = self.modulator.ul_slot_begin(tx_slot)
        {
            let block_first = block_count as SampleCount * buf.len() as SampleCount;
            let block_last = block_first + buf.len() as SampleCount;
            tracing::debug!(
                ts = %tx_slot.time,
                block_count,
                block_first,
                block_last,
                slot_begin,
                ahead_samples = slot_begin - block_first,
                "Modulator(Ul): pending burst vs TX generation window"
            );
            self.diag_last_time = Some(tx_slot.time);
        }

        while self.buffer_i < buf.len() {
            // TODO: include delay of FCFB in sample count
            match self.modulator.sample(
                block_count as SampleCount * buf.len() as SampleCount + self.buffer_i as SampleCount,
                tx_slot,
            ) {
                Ok(sample) => {
                    buf[self.buffer_i] = sample;
                    self.buffer_i += 1;
                }
                Err(modulator::Error::NeedMoreData) => {
                    return false;
                }
            }
        }
        fcfb.add(self.upconverter.process(self.buffer.buffer(), block_count));

        let _ = self.buffer.prepare_for_new_samples();
        self.buffer_i = 0;
        true
    }
}

#[cfg(test)]
mod rx_tests {
    use super::*;
    use crate::phy::components::soapy_time::{ticks_to_time_ns, time_ns_to_ticks};
    use std::collections::VecDeque;

    const CLOCK_ORIGIN: i64 = 1_234_567_890;
    const SAMPLE_RATE: f64 = 600_000.0;

    struct MockTransport {
        reads: VecDeque<(Option<SampleCount>, usize)>,
        next_count: SampleCount,
        requests: Vec<usize>,
        timestamps: Vec<i64>,
    }

    impl MockTransport {
        fn new(reads: impl IntoIterator<Item = (Option<SampleCount>, usize)>) -> Self {
            Self {
                reads: reads.into_iter().collect(),
                next_count: 0,
                requests: Vec::new(),
                timestamps: Vec::new(),
            }
        }

        fn receive(&mut self, buffer: &mut [ComplexSample]) -> Result<soapyio::RxResult, RxTxDevError> {
            self.requests.push(buffer.len());
            let (count, limit) = self.reads.pop_front().unwrap_or((None, usize::MAX));
            let timestamp = CLOCK_ORIGIN + ticks_to_time_ns(count.unwrap_or(self.next_count), SAMPLE_RATE);
            self.timestamps.push(timestamp);
            let count = time_ns_to_ticks(timestamp - CLOCK_ORIGIN, SAMPLE_RATE);
            let len = limit.min(buffer.len());
            for (i, sample) in buffer[..len].iter_mut().enumerate() {
                *sample = ComplexSample::new((count + i as i64) as f32, 0.0);
            }
            self.next_count = count + len as i64;
            Ok(soapyio::RxResult { count, len })
        }
    }

    fn dsp() -> RxDsp {
        let fcfb = fcfb::AnalysisInputProcessor::new(
            &mut FftPlanner::new(),
            fcfb::AnalysisInputParameters {
                fft_size: 1200,
                center_frequency: 430_425_000.0,
                sample_rate: SAMPLE_RATE,
                overlap: fcfb::Overlap::O1_4,
            },
        );
        let rx_block_size = fcfb.input_block_size();
        assert_eq!((rx_block_size.new, rx_block_size.overlap), (900, 300));
        RxDsp {
            rx_fcfb: fcfb,
            rx_block_size,
            rx_buffer: vec![ComplexSample::ZERO; 1200],
            rx_buffer_i: 0,
            rx_block_count: 0,
            downlinks: Vec::new(),
        }
    }

    fn assert_block(dsp: &RxDsp, beginning: SampleCount) {
        for (i, sample) in dsp.rx_buffer.iter().enumerate() {
            assert_eq!(sample.re, (beginning + i as i64) as f32, "sample {i}");
        }
    }

    #[test]
    fn gap_discard_larger_than_buffer_is_chunked_and_next_block_overlaps() {
        let mut dsp = dsp();
        let mut transport = MockTransport::new([(Some(1800), 100)]);
        dsp.receive_block_with(|buffer| transport.receive(buffer)).unwrap();
        assert_eq!(transport.requests, [900, 1200, 500, 1200]);
        assert_eq!(transport.timestamps[0] - CLOCK_ORIGIN, 3_000_000);
        assert_eq!(transport.timestamps[3] - CLOCK_ORIGIN, 6_000_000);
        assert_eq!(dsp.rx_block_count, 4);
        assert_block(&dsp, 3600);
        dsp.receive_block_with(|buffer| transport.receive(buffer)).unwrap();
        assert_eq!(dsp.rx_block_count, 5);
        assert_block(&dsp, 4500);
    }

    #[test]
    fn another_gap_during_discard_realigns_from_timestamp() {
        // End 5100 rounds up to block 6, then one block is skipped: 7*900=6300.
        // End 1800 is already on block 2, so the next complete block starts at 2700.
        for (second_count, expected_beginning) in [(5000, 6300), (1700, 2700)] {
            let mut dsp = dsp();
            let mut transport = MockTransport::new([(Some(1800), 100), (Some(second_count), 100)]);
            dsp.receive_block_with(|buffer| transport.receive(buffer)).unwrap();
            assert!(transport.requests.iter().all(|&len| len <= 1200 && len > 0));
            assert_eq!(dsp.rx_block_count, expected_beginning / 900);
            assert_block(&dsp, expected_beginning);
            assert_eq!(time_ns_to_ticks(transport.timestamps[1] - CLOCK_ORIGIN, SAMPLE_RATE), second_count);
        }
    }

    #[test]
    fn later_gap_while_filling_drops_partial_block() {
        let mut dsp = dsp();
        let mut transport = MockTransport::new([(Some(1800), 100), (None, 1200), (None, 500), (None, 100), (Some(9000), 80)]);
        dsp.receive_block_with(|buffer| transport.receive(buffer)).unwrap();
        assert_eq!(dsp.rx_block_count, 12);
        assert_block(&dsp, 10800);
    }

    #[test]
    fn zero_length_read_is_rejected_in_fill_and_discard() {
        for reads in [vec![(Some(1200), 0)], vec![(Some(1800), 100), (None, 0)]] {
            let mut dsp = dsp();
            let mut transport = MockTransport::new(reads);
            assert_eq!(
                dsp.receive_block_with(|buffer| transport.receive(buffer)),
                Err(RxTxDevError::RxReadError)
            );
        }
    }
}

/// Type used to represent input bits.
/// "0" is represented as -1, "1" as +1, and punctured bit as 0.
/// Soft decision decoding is also possible by using
/// higher negative values to represent more likely "0"
/// and higher positive values to represent more likely "1".
/// Too high values might cause path metrics to overflow though.
pub type SoftBit = i8;

/// Type used to accumulate path metrics.
/// 16 bits should be enough for our message lengths without need for renormalizations.
type Metric = i16;

/// Constraint length of the code.
/// This is defined as a constant rather than a const generic parameter
/// since it also affect NUM_STATES and the choice of type for DecisionBitmap
/// which would be more complicated to make generic.
/// If decoders for different constraint lengths are needed, it might be
/// easier to generate them using a macro instead.
const K: usize = 5;

const NUM_STATES: usize = num_states(K);

/// Unsigned integer type used to store decisions for each state in trellis.
/// Each bit represents a decision for a given state,
/// so the number of bits should be at least the number of states.
pub type DecisionBitmap = u16;

/// Caller-owned decoder storage requirements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    InvalidRate,
    InputTooShort,
    InvalidBit,
    SymbolCountTooLarge,
    OutputTooShort,
    TracebackTooShort,
}

/// Reusable caller-owned traceback storage. No soft-bit conversion array is
/// needed: hard bits are converted while the branch metrics are accumulated.
///
/// Initialize in caller-owned static storage with `const { Workspace::new() }`
/// when stack space is constrained, or use [`Self::initialize`] for runtime
/// storage. Decoding overwrites the used prefix, so no reset or replacement
/// workspace is needed between calls.
#[repr(align(4))]
pub struct Workspace<const SYMBOLS: usize> {
    decisions: [DecisionBitmap; SYMBOLS],
}

impl<const SYMBOLS: usize> Default for Workspace<SYMBOLS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SYMBOLS: usize> Workspace<SYMBOLS> {
    pub const fn new() -> Self {
        Self { decisions: [0; SYMBOLS] }
    }

    /// Initialize caller-owned runtime storage without constructing a second
    /// traceback array.
    #[allow(unsafe_code, reason = "initialize private integer-array scratch in place")]
    pub fn initialize(storage: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        // SAFETY: the sole field is an integer array, so all-zero bytes are a
        // valid initialized value. The exclusive storage borrow is correctly
        // aligned and covers the complete object, including empty workspaces.
        unsafe {
            storage.as_mut_ptr().write_bytes(0, 1);
            storage.assume_init_mut()
        }
    }
}

/// Caller-owned storage for the legacy maximum of 864 SB1 output symbols.
pub type Sb1Workspace = Workspace<864>;

/// Number of states for a given constraint length.
pub const fn num_states(k: usize) -> usize {
    1 << (k - 1)
}

/// Viterbi decoder for a binary convolutional code of rate 1/N.
pub struct ViterbiDecoder<const N: usize> {
    /// Expected encoder outputs for each state for encoder input "0".
    expected_0: [[SoftBit; NUM_STATES]; N],
}

impl<const N: usize> ViterbiDecoder<N> {
    pub fn new_with_polynomials(generator_polynomials: &[[bool; K]; N]) -> Self {
        // Generate look-up table for expected encoder output bits.
        // With the generator polynomials used here, the expected outputs
        // for "1" input are inverse of those for "0" input,
        // so only generate the table for "0" and invert results later.
        let expected_0 = core::array::from_fn(|poly_n| {
            let poly = generator_polynomials[poly_n];
            core::array::from_fn(|state| {
                let mut encoder_output: bool = false;
                // Each bit of the state number corresponds to
                // a past input of the encoder.
                for (bit_i, &tap) in poly.iter().take(K - 1).enumerate() {
                    let past_input_bit = (state & (1 << (K - 2 - bit_i))) != 0;
                    if past_input_bit && tap {
                        encoder_output = !encoder_output;
                    }
                }
                if encoder_output { 1 as SoftBit } else { -1 as SoftBit }
            })
        });
        Self { expected_0 }
    }

    /// Decode into caller-owned output and traceback storage.
    ///
    /// Both buffers need at least `received_bits.len() / N` entries. As in the
    /// original decoder, incomplete final symbols are ignored, ties choose the
    /// zero predecessor, and traceback starts at the terminated zero state.
    /// Only the written prefix of either buffer is modified.
    pub fn decode_into(
        &self,
        received_bits: &[SoftBit],
        decoded_bits: &mut [u8],
        trellis_decisions: &mut [DecisionBitmap],
    ) -> Result<usize, DecodeError> {
        if N == 0 {
            return Err(DecodeError::InvalidRate);
        }
        let num_output_bits = received_bits.len() / N;
        self.decode_symbols(num_output_bits, |i| received_bits[i], decoded_bits, trellis_decisions)
    }

    /// Decode soft bits using a reusable caller-owned workspace.
    pub fn decode_with_workspace<const SYMBOLS: usize>(
        &self,
        received_bits: &[SoftBit],
        decoded_bits: &mut [u8],
        workspace: &mut Workspace<SYMBOLS>,
    ) -> Result<usize, DecodeError> {
        self.decode_into(received_bits, decoded_bits, &mut workspace.decisions)
    }

    /// Decode hard bits (0, 1, or 0xff erasure) without a conversion array.
    ///
    /// As with `decode_into`, incomplete trailing symbols are ignored. Invalid
    /// consumed bits or insufficient output/traceback capacity are reported
    /// before output or workspace storage is modified.
    pub fn decode_hard_with_workspace<const SYMBOLS: usize>(
        &self,
        received_bits: &[u8],
        decoded_bits: &mut [u8],
        workspace: &mut Workspace<SYMBOLS>,
    ) -> Result<usize, DecodeError> {
        if N == 0 {
            return Err(DecodeError::InvalidRate);
        }
        let num_output_bits = received_bits.len() / N;
        if decoded_bits.len() < num_output_bits {
            return Err(DecodeError::OutputTooShort);
        }
        if SYMBOLS < num_output_bits {
            return Err(DecodeError::TracebackTooShort);
        }
        let received_bits = &received_bits[..num_output_bits * N];
        if received_bits.iter().any(|bit| !matches!(bit, 0 | 1 | 0xff)) {
            return Err(DecodeError::InvalidBit);
        }
        self.decode_symbols(
            num_output_bits,
            |i| match received_bits[i] {
                0 => -1,
                1 => 1,
                _ => 0,
            },
            decoded_bits,
            &mut workspace.decisions,
        )
    }

    fn decode_symbols(
        &self,
        num_output_bits: usize,
        received_bit: impl Fn(usize) -> SoftBit,
        decoded_bits: &mut [u8],
        trellis_decisions: &mut [DecisionBitmap],
    ) -> Result<usize, DecodeError> {
        if decoded_bits.len() < num_output_bits {
            return Err(DecodeError::OutputTooShort);
        }
        if trellis_decisions.len() < num_output_bits {
            return Err(DecodeError::TracebackTooShort);
        }

        // Accumulated path metrics for each state.
        //
        // Encoder starts from state 0. Give that an initial metric of 0.
        // Give a very high initial value for other metrics
        // so they will not be chosen.
        // Use half of maximum possible value so there is still room
        // to accumulate on top of it without overflow.
        let mut metrics: [Metric; NUM_STATES] = [Metric::MAX / 2; NUM_STATES];
        metrics[0] = 0;

        for (index, decision_slot) in trellis_decisions[..num_output_bits].iter_mut().enumerate() {
            // Branch metrics for encoder input "0".
            let mut branch_metrics_0: [Metric; NUM_STATES] = [0; NUM_STATES];

            // Loop through each generator polynomial and add to branch metrics
            for (polynomial, expected_0) in self.expected_0.iter().enumerate() {
                let received_bit = received_bit(index * N + polynomial);
                // Loop through each state
                for (branch_metric_0, expected_bit_0) in branch_metrics_0.iter_mut().zip(expected_0.iter()) {
                    *branch_metric_0 -= (received_bit * expected_bit_0) as Metric;
                }
            }

            let mut decisions: DecisionBitmap = 0;

            // New path metrics.
            metrics = core::array::from_fn(|state| {
                // Predecessor state if encoder input was 0.
                let predecessor_0 = (state * 2) % NUM_STATES;
                // Predecessor state if encoder input was 1.
                let predecessor_1 = predecessor_0 + 1;
                // Candidates for new path metrics.
                let metric_0 = metrics[predecessor_0] + branch_metrics_0[state];
                // With the generator polynomials used here, the expected
                // encoder outputs for encoder input "1" are inverse
                // of those for "0", so we can get branch metrics
                // for "1" input by using inverted metrics of "0" input.
                let metric_1 = metrics[predecessor_1] - branch_metrics_0[state];

                if metric_1 < metric_0 {
                    // We only need to store the decision as a single bit
                    // rather than the whole predecessor state number,
                    // since each state only has two possible predecessors.
                    // This is then converted back to state numbers in traceback.
                    decisions |= 1 << state;
                    metric_1
                } else {
                    metric_0
                }
            });
            *decision_slot = decisions;
        }

        // Traceback

        // Tail bits should ensure the final state of the encoder is 0.
        let mut best_state = 0;

        for index in (0..num_output_bits).rev() {
            let decisions = trellis_decisions[index];
            decoded_bits[index] = ((best_state >> (K - 2)) & 1) as u8;
            best_state = best_state * 2 % NUM_STATES + ((decisions >> best_state) & 1) as usize;
        }
        Ok(num_output_bits)
    }
}

/// Decoder for rate 1/4 mother code
pub type TetraViterbiDecoder = ViterbiDecoder<4>;

impl Default for TetraViterbiDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl TetraViterbiDecoder {
    pub fn new() -> Self {
        Self::new_with_polynomials(&[
            [true, true, false, false, true],
            [true, false, true, true, true],
            [true, true, true, false, true],
            [true, true, false, true, true],
        ])
    }
}

/// Decoder for rate 1/3 mother code used by TETRA speech codec
pub type TetraCodecViterbiDecoder = ViterbiDecoder<3>;

impl Default for TetraCodecViterbiDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl TetraCodecViterbiDecoder {
    pub fn new() -> Self {
        Self::new_with_polynomials(&[
            [true, true, true, true, true],
            [true, true, false, true, true],
            [true, false, true, false, true],
        ])
    }
}

/// Decode at most 864 SB1 output symbols using caller-owned traceback storage.
///
/// Only `sym_count * 4` input entries are consumed, matching the desktop entry
/// point. No large scratch array is constructed in this function.
pub fn dec_sb1_with_workspace<const SYMBOLS: usize>(
    in_buf: &[u8],
    out_buf: &mut [u8],
    sym_count: usize,
    workspace: &mut Workspace<SYMBOLS>,
) -> Result<(), DecodeError> {
    if sym_count > 864 {
        return Err(DecodeError::SymbolCountTooLarge);
    }
    if in_buf.len() < sym_count * 4 {
        return Err(DecodeError::InputTooShort);
    }
    TetraViterbiDecoder::new()
        .decode_hard_with_workspace(&in_buf[..sym_count * 4], out_buf, workspace)
        .map(|_| ())
}

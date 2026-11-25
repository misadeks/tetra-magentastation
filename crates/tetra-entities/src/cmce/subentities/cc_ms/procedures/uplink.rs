use super::*;
use tetra_ms_core::cmce::CmceEvent;
impl CcMsSubentity {
    pub fn request_tx(&mut self, queue: &mut MessageQueue, id: u16, priority: u8) -> bool {
        self.dispatch(queue, CmceEvent::Demand { id, priority })
    }
    pub fn cease_tx(&mut self, queue: &mut MessageQueue, id: u16) -> bool {
        self.dispatch(queue, CmceEvent::Cease { id })
    }
    pub fn handle_tncc_dtmf(&mut self, queue: &mut MessageQueue, id: u16, request: &tncc::TnccDtmfRequest) -> Result<(), String> {
        let digits = request.dtmf_digits.as_deref().unwrap_or(&[]);
        if request.dtmf_tone_delimiter == tncc::DtmfToneDelimiter::Dtmf
            && request
                .number_of_dtmf_digits
                .is_some_and(|count| usize::from(count) != digits.len())
        {
            return Err("TNCC-DTMF digit count disagrees with supplied digits".to_owned());
        }
        let nibbles: Vec<u8> = digits.iter().map(|digit| dtmf_digit_nibble(*digit)).collect();
        let data = tetra_ms_core::cmce::encode_dtmf(if request.dtmf_tone_delimiter == tncc::DtmfToneDelimiter::Dtmf {
            Some(&nibbles)
        } else {
            None
        })
        .map_err(|error| format!("invalid DTMF: {error:?}"))?;
        if self.dispatch(queue, CmceEvent::Dtmf { id, data: data.view() }) {
            Ok(())
        } else {
            Err("CMCE DTMF rejected".to_owned())
        }
    }
}
fn dtmf_digit_nibble(digit: tncc::DtmfDigit) -> u8 {
    match digit {
        tncc::DtmfDigit::Digit0 => 0,
        tncc::DtmfDigit::Digit1 => 1,
        tncc::DtmfDigit::Digit2 => 2,
        tncc::DtmfDigit::Digit3 => 3,
        tncc::DtmfDigit::Digit4 => 4,
        tncc::DtmfDigit::Digit5 => 5,
        tncc::DtmfDigit::Digit6 => 6,
        tncc::DtmfDigit::Digit7 => 7,
        tncc::DtmfDigit::Digit8 => 8,
        tncc::DtmfDigit::Digit9 => 9,
        tncc::DtmfDigit::DigitStar => 10,
        tncc::DtmfDigit::DigitHash => 11,
        tncc::DtmfDigit::DigitA => 12,
        tncc::DtmfDigit::DigitB => 13,
        tncc::DtmfDigit::DigitC => 14,
        tncc::DtmfDigit::DigitD => 15,
    }
}

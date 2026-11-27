use core::fmt;

use tetra_core::pdu_parse_error::PduParseErr;
use tetra_core::{BitBuffer, expect_failed, expect_value};

use crate::umac::enums::reservation_requirement::ReservationRequirement;

/// Clause 21.4.2.5 MAC-END (uplink)
#[derive(Debug, Clone)]
pub struct MacEndUl {
    // 1
    pub fill_bits: bool,
    // 6
    // If 2-bits length_ind_cap_req < 0b11, field holds 6-bit length indication
    pub length_ind: Option<u8>,
    // If 2-bits length_ind_cap_req == 0b11, then reservation_req field holds 4  data bits
    pub reservation_req: Option<ReservationRequirement>,
}

impl MacEndUl {
    pub fn from_bitbuf(buf: &mut BitBuffer) -> Result<Self, PduParseErr> {
        // required constant mac_pdu_type
        let mac_pdu_type = buf.read_field(2, "mac_pdu_type")?;
        expect_value!(mac_pdu_type, 1, "mac_pdu_type")?;
        // required constant pdu_subtype
        let pdu_subtype = buf.read_field(1, "pdu_subtype")?;
        expect_value!(pdu_subtype, 1, "pdu_subtype")?;
        let fill_bits = buf.read_field(1, "fill_bits")? != 0;
        let length_ind_cap_req = buf.read_field(6, "length_ind_cap_req")?;
        let (length_ind, reservation_req) = if length_ind_cap_req == 0 {
            // Reserved value
            return expect_failed!(length_ind_cap_req, "length_ind_cap_req reserved value");
        } else if length_ind_cap_req < 0b101111 {
            // Length indication
            (Some(length_ind_cap_req as u8), None)
        } else if length_ind_cap_req < 0b110000 {
            // reserved value, return error
            return expect_failed!(length_ind_cap_req, "length_ind_cap_req reserved value");
        } else {
            // 0b110000 or higher, capacity request
            let val = length_ind_cap_req & 0b001111;
            let res_req = ReservationRequirement::try_from(val).map_err(|_| PduParseErr::InvalidValue {
                field: "reservation_req",
                value: val,
            })?;
            (None, Some(res_req))
        };

        Ok(MacEndUl {
            fill_bits,
            length_ind,
            reservation_req,
        })
    }

    pub fn to_bitbuf(&self, buf: &mut BitBuffer) -> Result<(), PduParseErr> {
        expect_value!(
            self.length_ind.is_some() ^ self.reservation_req.is_some(),
            true,
            "length_ind xor reservation_req must be present"
        )?;
        if let Some(length_ind) = self.length_ind {
            expect_value!(length_ind > 0, true, "length_ind zero")?;
            expect_value!(length_ind <= 0b101110, true, "length_ind over 0b101110")?;
        }
        // write required constant mac_pdu_type
        buf.write_bits(1, 2);
        // write required constant pdu_subtype
        buf.write_bits(1, 1);
        buf.write_bits(self.fill_bits as u8 as u64, 1);
        if let Some(length_ind) = self.length_ind {
            buf.write_bits(length_ind as u64, 6);
        } else if let Some(reservation_req) = self.reservation_req {
            // assert!(reservation_req < 0b001111);
            buf.write_bits(0b11, 2);
            buf.write_bits(reservation_req as u64, 4);
        }
        Ok(())
    }
}

impl fmt::Display for MacEndUl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MacEndUl {{ fill_bits: {}", self.fill_bits)?;
        if let Some(length_ind) = self.length_ind {
            write!(f, "  length_ind: {}", length_ind)?;
        }
        if let Some(reservation_req) = self.reservation_req {
            write!(f, "  reservation_req: {}", reservation_req)?;
        }
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regression_all_length_and_reservation_codes_roundtrip_or_reject_reserved() {
        for fill_bits in [false, true] {
            for raw in 0..64 {
                let wire = format!("011{}{:06b}", u8::from(fill_bits), raw);
                let mut input = BitBuffer::from_bitstr(&wire);
                let parsed = MacEndUl::from_bitbuf(&mut input);
                if raw == 0 || raw == 47 {
                    assert!(matches!(parsed, Err(PduParseErr::InvalidValue { .. })));
                    continue;
                }
                let pdu = parsed.unwrap();
                assert_eq!(pdu.fill_bits, fill_bits);
                if raw < 48 {
                    assert_eq!(pdu.length_ind, Some(raw as u8));
                    assert_eq!(pdu.reservation_req, None);
                } else {
                    assert_eq!(pdu.length_ind, None);
                    assert_eq!(pdu.reservation_req.unwrap().into_raw(), raw & 0b001111);
                }
                assert_eq!(input.get_len_remaining(), 0);
                let mut output = BitBuffer::new(10);
                pdu.to_bitbuf(&mut output).unwrap();
                assert_eq!(output.to_bitstr(), wire);
            }
        }
    }

    #[test]
    fn regression_malformed_headers_and_truncated_pdus_return_errors() {
        for header in 0..8 {
            if header == 0b011 {
                continue;
            }
            let mut input = BitBuffer::from_bitstr(&format!("{header:03b}0000001"));
            assert!(MacEndUl::from_bitbuf(&mut input).is_err());
        }
        for len in 0..10 {
            let mut input = BitBuffer::from_bitstr(&"0110110001"[..len]);
            assert!(MacEndUl::from_bitbuf(&mut input).is_err());
        }
    }

    #[test]
    fn regression_invalid_encoder_values_do_not_write_partial_headers() {
        let mut invalid = vec![
            MacEndUl {
                fill_bits: false,
                length_ind: None,
                reservation_req: None,
            },
            MacEndUl {
                fill_bits: true,
                length_ind: Some(1),
                reservation_req: Some(ReservationRequirement::Req1Slot),
            },
        ];
        invalid.extend([0, 47, 48, 63, 255].map(|length| MacEndUl {
            fill_bits: false,
            length_ind: Some(length),
            reservation_req: None,
        }));
        for pdu in invalid {
            let mut output = BitBuffer::from_bitstr("101");
            output.seek(3);
            assert!(pdu.to_bitbuf(&mut output).is_err());
            assert_eq!(output.get_pos(), 3);
            assert_eq!(output.to_bitstr(), "101");
        }
    }
}

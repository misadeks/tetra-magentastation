use super::*;
use crate::umac::subcomp::fillbits;
use tetra_pdus::umac::enums::reservation_requirement::ReservationRequirement;
use tetra_pdus::umac::pdus::{
    mac_access::MacAccess, mac_data::MacData, mac_end_hu::MacEndHu, mac_end_ul::MacEndUl, mac_resource::MacResource,
};

fn sdu(len: usize) -> BitBuffer {
    let mut buffer = BitBuffer::new(len);
    for i in 0..len {
        buffer.write_bit(u8::from((i * 13 + i / 7) % 5 < 2));
    }
    buffer.seek(0);
    buffer
}

fn assert_bits(mut actual: BitBuffer, mut expected: BitBuffer) {
    assert_eq!(actual.get_len(), expected.get_len());
    actual.seek(0);
    expected.seek(0);
    assert_eq!(actual.dump_bin(), expected.dump_bin());
}

#[test]
fn portable_uplink_bits_match_original_codecs_at_all_fill_boundaries() {
    let issi = 1_000_001;
    for len in 0..=90 {
        let mut source = sdu(len);
        let actual = UmacMs::build_mac_access_block(issi, &mut source);
        if len <= 62 {
            let mut expected = BitBuffer::new(92);
            MacAccess {
                fill_bits: len != 62,
                encrypted: false,
                addr: Some(TetraAddress {
                    ssi: issi,
                    ssi_type: tetra_core::SsiType::Issi,
                }),
                event_label: None,
                length_ind: None,
                frag_flag: None,
                reservation_req: None,
            }
            .to_bitbuf(&mut expected);
            expected.copy_bits(&mut sdu(len), len);
            fillbits::addition::write(&mut expected, None);
            assert_bits(actual.unwrap(), expected);
        } else {
            assert!(actual.is_none());
        }

        let actual = UmacMs::build_stch_mac_data_block(issi, &mut sdu(len));
        let mut pdu = MacData {
            fill_bits: false,
            encrypted: false,
            addr: Some(TetraAddress {
                ssi: issi,
                ssi_type: tetra_core::SsiType::Issi,
            }),
            event_label: None,
            length_ind: Some(0),
            frag_flag: None,
            reservation_req: None,
        };
        let fill = pdu.update_len_and_fill_ind(len);
        if usize::from(pdu.length_ind.unwrap()) * 8 <= 124 {
            let mut expected = BitBuffer::new(124);
            pdu.to_bitbuf(&mut expected);
            expected.copy_bits(&mut sdu(len), len);
            fillbits::addition::write(&mut expected, Some(fill));
            assert_bits(actual.unwrap(), expected);
        } else {
            assert!(actual.is_none());
        }
    }
    for len in 0..=260 {
        for half in [true, false] {
            let (header, capacity) = if half { (7, 92) } else { (10, 268) };
            let fill = fillbits::addition::compute_required_bytealigned(header + len);
            let actual = if half {
                UmacMs::build_mac_end_hu_block(&mut sdu(len))
            } else {
                UmacMs::build_mac_end_ul_block(&mut sdu(len))
            };
            if header + len + fill > capacity {
                assert!(actual.is_none());
                continue;
            }
            let mut expected = BitBuffer::new(capacity);
            let length = Some(((header + len + fill) / 8) as u8);
            if half {
                MacEndHu {
                    fill_bits: fill != 0,
                    length_ind: length,
                    reservation_req: None,
                }
                .to_bitbuf(&mut expected);
            } else {
                MacEndUl {
                    fill_bits: fill != 0,
                    length_ind: length,
                    reservation_req: None,
                }
                .to_bitbuf(&mut expected)
                .unwrap();
            }
            expected.copy_bits(&mut sdu(len), len);
            fillbits::addition::write(&mut expected, Some(fill));
            expected.write_zeroes(expected.get_len_remaining());
            assert_bits(actual.unwrap(), expected);
        }
    }
}

#[test]
fn portable_fragment_start_bits_and_remainders_match_original_codec() {
    for len in 57..=1640 {
        let actual = UmacMs::build_mac_access_frag_start(1_000_001, &mut sdu(len));
        let remainder = len - 56;
        if remainder > 5 * 264 + 254 {
            assert!(actual.is_none());
            continue;
        }
        let (actual, tail, kind) = actual.unwrap();
        let slots = match kind {
            FragEndKind::MacEndHu => 0,
            FragEndKind::MacEndUl => 1,
            FragEndKind::MacFragUl { total_slots } => total_slots,
        };
        let mut expected = BitBuffer::new(92);
        MacAccess {
            fill_bits: false,
            encrypted: false,
            addr: Some(TetraAddress {
                ssi: 1_000_001,
                ssi_type: tetra_core::SsiType::Issi,
            }),
            event_label: None,
            length_ind: None,
            frag_flag: Some(true),
            reservation_req: Some(ReservationRequirement::from_req_slotcount(usize::from(slots))),
        }
        .to_bitbuf(&mut expected);
        let mut source = sdu(len);
        expected.copy_bits(&mut source, 56);
        assert_bits(actual, expected);
        assert_bits(tail, BitBuffer::from_bitbuffer_pos(&source));
    }
}

#[test]
fn portable_resource_headers_match_original_address_and_optional_layouts() {
    for addr_type in 0..8 {
        for flags in 0..8 {
            let address = match addr_type {
                0 | 2 => None,
                3 => Some(TetraAddress {
                    ssi: 12,
                    ssi_type: tetra_core::SsiType::Ussi,
                }),
                4 | 7 => Some(TetraAddress {
                    ssi: 12,
                    ssi_type: tetra_core::SsiType::Smi,
                }),
                _ => Some(TetraAddress {
                    ssi: 12,
                    ssi_type: tetra_core::SsiType::Ssi,
                }),
            };
            let mut pdu = MacResource {
                addr: address,
                event_label: matches!(addr_type, 2 | 5 | 7).then_some(22),
                usage_marker: (addr_type == 6).then_some(16),
                length_ind: 16,
                power_control_element: (flags & 1 != 0).then_some(3),
                slot_granting_element: (flags & 2 != 0).then_some(BasicSlotgrant {
                    capacity_allocation: BasicSlotgrantCapAlloc::Grant2Slots,
                    granting_delay: BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
                }),
                chan_alloc_element: (flags & 4 != 0).then_some(ChanAllocElement {
                    alloc_type: ChanAllocType::Additional,
                    ts_assigned: [false, true, false, false],
                    ul_dl_assigned: UlDlAssignment::Both,
                    clch_permission: false,
                    cell_change_flag: false,
                    carrier_num: 1593,
                    ext: None,
                    mon_pattern: 0,
                    frame18_mon_pattern: Some(2),
                }),
                ..MacResource::default()
            };
            if addr_type == 0 {
                pdu.power_control_element = None;
                pdu.slot_granting_element = None;
                pdu.chan_alloc_element = None;
            }

            let mut block = BitBuffer::new(268);
            pdu.to_bitbuf(&mut block);
            block.seek(0);
            let legacy = MacResource::from_bitbuf(&mut block).unwrap();
            let decoded = mac::downlink::resource(pack::<34>(&block).unwrap().view()).unwrap();
            assert_eq!(decoded.header_bits as usize, block.get_pos());
            assert_eq!(decoded.addr, legacy.addr);
            assert_eq!(decoded.event_label, legacy.event_label);
            assert_eq!(decoded.usage_marker, legacy.usage_marker);
            assert_eq!(decoded.grant.is_some(), legacy.slot_granting_element.is_some());
            assert_eq!(decoded.allocation.is_some(), legacy.chan_alloc_element.is_some());
        }
    }
}

#[test]
fn portable_aach_matches_original_parser_for_every_wire_pattern() {
    use tetra_pdus::umac::pdus::access_assign_fr18::AccessAssignFr18;
    for raw in 0..1_u32 << 14 {
        let mut buffer = BitBuffer::new(14);
        buffer.write_bits(u64::from(raw), 14);
        buffer.seek(0);
        let bits = pack::<34>(&buffer).unwrap();
        let legacy = AccessAssign::from_bitbuf(&mut buffer);
        let portable = mac::downlink::aach(bits.view(), false);
        assert_eq!(legacy.is_ok(), portable.is_ok(), "AACH {raw:014b}");
        if let (Ok(legacy), Ok(portable)) = (legacy, portable) {
            assert_eq!(portable.traffic_marker, legacy.dl_usage.get_tchan());
            assert_eq!(portable.access, Some(interpret_access_assign(&legacy, true)));
        }
        buffer.seek(0);
        assert_eq!(
            AccessAssignFr18::from_bitbuf(&mut buffer).is_ok(),
            mac::downlink::aach(bits.view(), true).is_ok(),
            "frame18 AACH {raw:014b}"
        );
    }
}

#[test]
fn portable_broadcast_headers_match_original_option_and_restriction_decoding() {
    use tetra_pdus::umac::pdus::mac_sysinfo::MacSysinfo;
    for option in 0..4 {
        for cck in 0..2 {
            for raw_option in [0, 0xabcde, 0xfffff] {
                let mut buffer = BitBuffer::new(124);
                for (value, width) in [
                    (8, 4),
                    (1593, 12),
                    (4, 4),
                    (3, 2),
                    (7, 3),
                    (1, 1),
                    (0x12345, 17),
                    (cck, 1),
                    (77, 16),
                    (option, 2),
                    (raw_option, 20),
                ] {
                    buffer.write_bits(value, width);
                }
                buffer.seek(0);
                let legacy = MacSysinfo::from_bitbuf(&mut buffer).unwrap();
                let portable = mac::downlink::sysinfo(pack::<34>(&buffer).unwrap().view()).unwrap();
                assert_eq!(portable.sdu_offset as usize, buffer.get_pos());
                assert_eq!(portable.main_carrier, legacy.main_carrier);
                assert_eq!(portable.freq_band, legacy.freq_band);
                assert_eq!(portable.freq_offset_index, legacy.freq_offset_index);
                assert_eq!(portable.duplex_spacing, legacy.duplex_spacing);
                assert_eq!(portable.reverse_operation, legacy.reverse_operation);
                assert_eq!(portable.hyperframe, legacy.hyperframe_number);
                assert_eq!(portable.option_bits as u64, raw_option);
                assert_eq!(portable.default_access.is_some(), legacy.default_access_code.is_some());
                if let (Some(portable), Some(legacy)) = (portable.default_access, legacy.default_access_code) {
                    assert_eq!(
                        [portable.imm, portable.wt, portable.nu, portable.ts_ptr, portable.min_pdu_prio],
                        [legacy.imm, legacy.wt, legacy.nu, legacy.ts_ptr, legacy.min_pdu_prio]
                    );
                    assert_eq!(portable.fl_factor, legacy.fl_factor);
                }
            }
        }
    }
    for option in 0..4 {
        for code in 0..4 {
            for assigned in [false, true] {
                let pdu = AccessDefine {
                    common_or_assigned_control: assigned,
                    access_code: code,
                    imm: 15,
                    wt: 6,
                    nu: 4,
                    frame_len_factor: true,
                    ts_pointer: 7,
                    min_pdu_prio: 5,
                    opt_field_flag: option,
                    subscriber_class: (option == 1).then_some(0x1234),
                    gssi: (option == 2).then_some(0xabcdef),
                };
                let mut buffer = BitBuffer::new(124);
                pdu.to_bitbuf(&mut buffer);
                buffer.seek(0);
                let legacy = AccessDefine::from_bitbuf(&mut buffer).unwrap();
                let portable = mac::downlink::access_define(pack::<34>(&buffer).unwrap().view()).unwrap();
                assert_eq!(portable.assigned_control, legacy.common_or_assigned_control);
                assert_eq!(portable.code as u8, legacy.access_code);
                assert_eq!(portable.params.subscriber_class, legacy.subscriber_class);
                assert_eq!(portable.params.gssi, legacy.gssi);
                assert_eq!(portable.params.min_pdu_prio, legacy.min_pdu_prio);
            }
        }
    }
}

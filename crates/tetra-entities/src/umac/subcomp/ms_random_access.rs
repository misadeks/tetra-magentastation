//! Application PDU conversion and entropy source for portable random access.
use tetra_core::TdmaTime;
use tetra_ms_core::random_access as portable;
use tetra_pdus::umac::enums::access_assign_ul_usage::AccessAssignUlUsage;
use tetra_pdus::umac::fields::sysinfo_default_def_for_access_code_a::SysinfoDefaultDefForAccessCodeA;
use tetra_pdus::umac::pdus::access_assign::{AccessAssign, AccessField};
use tetra_pdus::umac::pdus::access_define::AccessDefine;

pub use portable::{AccessCode, BaseFrameLength, RaAction, RaFailure, RandomAccessParams, SlotAccessAssign, Subslot, SubslotAccess};

fn from_sysinfo(def: &SysinfoDefaultDefForAccessCodeA) -> RandomAccessParams {
    RandomAccessParams {
        imm: def.imm,
        wt: def.wt,
        nu: def.nu,
        fl_factor: def.fl_factor,
        ts_ptr: def.ts_ptr,
        min_pdu_prio: def.min_pdu_prio,
        subscriber_class: None,
        gssi: None,
    }
}
fn from_define(def: &AccessDefine) -> RandomAccessParams {
    RandomAccessParams {
        imm: def.imm,
        wt: def.wt,
        nu: def.nu,
        fl_factor: def.frame_len_factor,
        ts_ptr: def.ts_pointer,
        min_pdu_prio: def.min_pdu_prio,
        subscriber_class: def.subscriber_class,
        gssi: def.gssi,
    }
}

#[derive(Debug, Default, Clone)]
pub struct AccessParamStore(portable::AccessParamStore);
impl AccessParamStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn update_sysinfo_default_a(&mut self, def: &SysinfoDefaultDefForAccessCodeA) {
        self.0.update_sysinfo_default_a(from_sysinfo(def));
    }
    pub fn update_access_define(&mut self, def: &AccessDefine) {
        match AccessCode::from_raw(def.access_code) {
            Some(code) => self.0.update_access_define(code, from_define(def)),
            None => tracing::warn!(code = def.access_code, "Invalid access code"),
        }
    }
    pub fn params_for(&self, code: AccessCode) -> Option<&RandomAccessParams> {
        self.0.params_for(code)
    }
}

pub fn interpret_access_assign(aa: &AccessAssign, common: bool) -> SlotAccessAssign {
    let field = |value: AccessField| portable::AccessField {
        access_code: value.access_code,
        base_frame_len: value.base_frame_len,
    };
    portable::interpret_access_assign(
        &portable::AccessAssign {
            ul_usage: if aa.ul_usage == AccessAssignUlUsage::AssignedOnly {
                portable::AccessAssignUlUsage::AssignedOnly
            } else {
                portable::AccessAssignUlUsage::Other
            },
            f1_af1: aa.f1_af1.map(field),
            f2_af2: aa.f2_af2.map(field),
            f2_af: aa.f2_af.map(field),
        },
        common,
    )
}

/// Source of the random choices required by the random access algorithm
/// (ETSI TS 100 392-2 cl. 23.5.1.4.5 and .4.6). Injected so the state machine is
/// deterministically testable.
pub trait RaRng {
    /// Choose a subslot index uniformly in `1..=frame_length`
    /// (cl. 23.5.1.4.6). `frame_length` is always ≥ 1.
    fn choose_subslot_index(&mut self, frame_length: u16) -> u16;
    /// Choose one of the two subslots when both are valid access opportunities
    /// in the first-try slot (cl. 23.5.1.4.5).
    fn choose_one_of_two(&mut self) -> Subslot;
}

/// Default [`RaRng`] backed by the `rand` crate (matches the repo's existing
/// `rand::random_range` usage).
pub struct ThreadRaRng;

impl RaRng for ThreadRaRng {
    fn choose_subslot_index(&mut self, frame_length: u16) -> u16 {
        rand::random_range(1..=frame_length)
    }
    fn choose_one_of_two(&mut self) -> Subslot {
        if rand::random_range(0..2) == 0 {
            Subslot::One
        } else {
            Subslot::Two
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct MsRandomAccess(portable::MsRandomAccess);
impl core::ops::Deref for MsRandomAccess {
    type Target = portable::MsRandomAccess;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl core::ops::DerefMut for MsRandomAccess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl MsRandomAccess {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn poll_downlink_slot(
        &mut self,
        time: TdmaTime,
        assign: &SlotAccessAssign,
        valid: bool,
        params: &RandomAccessParams,
        rng: &mut dyn RaRng,
    ) -> Option<RaAction> {
        let (index, subslot) = match self.0.entropy_needed(time, assign, valid, params) {
            Ok(Some(portable::EntropyRequest::SubslotIndex(length))) => (rng.choose_subslot_index(length), Subslot::One),
            Ok(Some(portable::EntropyRequest::OneOfTwo)) => (1, rng.choose_one_of_two()),
            Ok(None) => (1, Subslot::One),
            Err(error) => {
                tracing::warn!(?error, "MS random access input rejected");
                return None;
            }
        };
        match self
            .0
            .poll_downlink_slot(time, assign, valid, params, &mut portable::RaChoices::new(index, subslot))
        {
            Ok(action) => action,
            Err(error) => {
                tracing::warn!(?error, "MS random access entropy rejected");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tetra_pdus::umac::enums::access_assign_dl_usage::AccessAssignDlUsage;

    use super::*;

    fn af(code: u8, base: u8) -> AccessField {
        AccessField {
            access_code: code,
            base_frame_len: base,
        }
    }

    fn aa_two_fields(af1: AccessField, af2: AccessField) -> AccessAssign {
        AccessAssign {
            _header: 0,
            dl_usage: AccessAssignDlUsage::CommonControl,
            ul_usage: AccessAssignUlUsage::CommonOnly,
            f1_af1: Some(af1),
            f2_af2: Some(af2),
            f2_af: None,
        }
    }

    fn aa_single_field(field: AccessField, ul_usage: AccessAssignUlUsage) -> AccessAssign {
        AccessAssign {
            _header: 1,
            dl_usage: AccessAssignDlUsage::CommonControl,
            ul_usage,
            f1_af1: None,
            f2_af2: None,
            f2_af: Some(field),
        }
    }
    fn sysinfo_def_a() -> SysinfoDefaultDefForAccessCodeA {
        SysinfoDefaultDefForAccessCodeA {
            imm: 3,
            wt: 5,
            nu: 4,
            fl_factor: false,
            ts_ptr: 0,
            min_pdu_prio: 0,
        }
    }

    fn access_define(code: u8, nu: u8, min_prio: u8) -> AccessDefine {
        AccessDefine {
            common_or_assigned_control: false,
            access_code: code,
            imm: 15,
            wt: 6,
            nu,
            frame_len_factor: true,
            ts_pointer: 1,
            min_pdu_prio: min_prio,
            opt_field_flag: 0,
            subscriber_class: None,
            gssi: None,
        }
    }

    #[test]
    fn test_base_frame_length_table() {
        // Table 21.85 verbatim.
        assert_eq!(BaseFrameLength::from_raw(0b0000), BaseFrameLength::ReservedSubslot);
        assert_eq!(BaseFrameLength::from_raw(0b0001), BaseFrameLength::ClchSubslot);
        assert_eq!(BaseFrameLength::from_raw(0b0010), BaseFrameLength::OngoingFrame);
        assert_eq!(BaseFrameLength::from_raw(0b0011), BaseFrameLength::FrameMarker(1));
        assert_eq!(BaseFrameLength::from_raw(0b0111), BaseFrameLength::FrameMarker(5));
        assert_eq!(BaseFrameLength::from_raw(0b1000), BaseFrameLength::FrameMarker(6));
        assert_eq!(BaseFrameLength::from_raw(0b1001), BaseFrameLength::FrameMarker(8));
        assert_eq!(BaseFrameLength::from_raw(0b1111), BaseFrameLength::FrameMarker(32));

        assert!(BaseFrameLength::from_raw(0b0011).is_frame_marker());
        assert!(!BaseFrameLength::from_raw(0b0010).is_frame_marker());
        assert_eq!(BaseFrameLength::from_raw(0b0110).base_subslots(), Some(4));
        assert_eq!(BaseFrameLength::from_raw(0b0010).base_subslots(), None);
        assert!(BaseFrameLength::from_raw(0b0000).is_reserved());
        assert!(BaseFrameLength::from_raw(0b0001).is_reserved());
        assert!(!BaseFrameLength::from_raw(0b0010).is_reserved());
    }

    #[test]
    fn test_sysinfo_default_a_until_access_define() {
        let mut store = AccessParamStore::new();
        // No definition yet.
        assert!(store.params_for(AccessCode::A).is_none());

        // SYSINFO default definition applies to code A (cl. 23.5.1.4.10).
        store.update_sysinfo_default_a(&sysinfo_def_a());
        let p = store.params_for(AccessCode::A).expect("code A available from SYSINFO");
        assert_eq!(p.imm, 3);
        assert_eq!(p.nu, 4);
        assert_eq!(p.subscriber_class, None);

        // A "common" ACCESS-DEFINE for code A overrides the SYSINFO default.
        store.update_access_define(&access_define(0, 7, 2));
        let p = store.params_for(AccessCode::A).expect("code A from ACCESS-DEFINE");
        assert_eq!(p.imm, 15, "ACCESS-DEFINE params now in force");
        assert_eq!(p.nu, 7);
        assert_eq!(p.min_pdu_prio, 2);
        assert!(p.fl_factor);
    }

    #[test]
    fn test_codes_bcd_require_access_define() {
        let mut store = AccessParamStore::new();
        store.update_sysinfo_default_a(&sysinfo_def_a());

        // SYSINFO default only covers code A; B/C/D need ACCESS-DEFINE.
        assert!(store.params_for(AccessCode::B).is_none());
        assert!(store.params_for(AccessCode::C).is_none());
        assert!(store.params_for(AccessCode::D).is_none());

        store.update_access_define(&access_define(1, 3, 0));
        assert!(store.params_for(AccessCode::B).is_some());
        assert!(store.params_for(AccessCode::C).is_none());
    }

    #[test]
    fn test_nu_zero_is_unavailable() {
        let p = from_define(&access_define(0, 0, 0));
        assert!(!p.is_available(), "Nu == 0 means access code not available (cl. 23.5.1.4.1)");
        let p = from_define(&access_define(0, 1, 0));
        assert!(p.is_available());
    }

    #[test]
    fn test_interpret_two_access_fields() {
        // Header 0: field 1 → subslot 1, field 2 → subslot 2, independently
        // (cl. 23.5.1.4.2). Code B (01), 2-subslot marker in subslot 1; code A
        // (00) ongoing frame in subslot 2.
        let aa = aa_two_fields(af(0b01, 0b0100), af(0b00, 0b0010));
        let s = interpret_access_assign(&aa, true);
        assert_eq!(s.subslot1.access_code, Some(AccessCode::B));
        assert_eq!(s.subslot1.frame_len, BaseFrameLength::FrameMarker(2));
        assert!(s.subslot1.is_frame_marker_for(AccessCode::B));
        assert!(s.subslot1.is_opportunity_for(AccessCode::B));
        assert!(!s.subslot1.is_opportunity_for(AccessCode::A));

        assert_eq!(s.subslot2.access_code, Some(AccessCode::A));
        assert_eq!(s.subslot2.frame_len, BaseFrameLength::OngoingFrame);
        assert!(s.subslot2.is_opportunity_for(AccessCode::A));
        assert!(!s.subslot2.is_frame_marker_for(AccessCode::A));
    }

    #[test]
    fn test_interpret_single_field_reserved_and_clch() {
        // Point b) reserved → both subslots reserved.
        let s = interpret_access_assign(&aa_single_field(af(0, 0b0000), AccessAssignUlUsage::CommonAndAssigned), true);
        assert!(s.subslot1.frame_len.is_reserved());
        assert!(s.subslot2.frame_len.is_reserved());
        assert!(!s.subslot1.is_opportunity_for(AccessCode::A));

        // Point c) CLCH → subslot 1 linearization (not an opportunity), subslot
        // 2 reserved.
        let s = interpret_access_assign(&aa_single_field(af(0, 0b0001), AccessAssignUlUsage::CommonAndAssigned), true);
        assert_eq!(s.subslot1.frame_len, BaseFrameLength::ClchSubslot);
        assert!(!s.subslot1.is_opportunity_for(AccessCode::A));
        assert_eq!(s.subslot2.frame_len, BaseFrameLength::ReservedSubslot);
    }

    #[test]
    fn test_interpret_single_field_marker_and_ongoing() {
        // Point e) frame marker → subslot 1 marker, subslot 2 ongoing frame,
        // both for the same access code (code C = 10, 3-subslot marker = 0101).
        let s = interpret_access_assign(&aa_single_field(af(0b10, 0b0101), AccessAssignUlUsage::CommonAndAssigned), true);
        assert_eq!(s.subslot1.access_code, Some(AccessCode::C));
        assert_eq!(s.subslot1.frame_len, BaseFrameLength::FrameMarker(3));
        assert!(s.subslot1.is_frame_marker_for(AccessCode::C));
        assert_eq!(s.subslot2.access_code, Some(AccessCode::C));
        assert_eq!(s.subslot2.frame_len, BaseFrameLength::OngoingFrame);
        assert!(s.subslot2.is_opportunity_for(AccessCode::C));
        assert!(!s.subslot2.is_frame_marker_for(AccessCode::C));

        // Point d) ongoing frame → both subslots ongoing for the same code.
        let s = interpret_access_assign(&aa_single_field(af(0b00, 0b0010), AccessAssignUlUsage::CommonAndAssigned), true);
        assert!(s.subslot1.is_opportunity_for(AccessCode::A));
        assert!(s.subslot2.is_opportunity_for(AccessCode::A));
    }

    #[test]
    fn test_interpret_assigned_only_on_common_is_reserved() {
        // An MS on the CCCH treats "Assigned only" as both subslots reserved
        // (cl. 23.5.1.4.2), even though the field would otherwise be usable.
        let s = interpret_access_assign(&aa_single_field(af(0b00, 0b0100), AccessAssignUlUsage::AssignedOnly), true);
        assert!(s.subslot1.frame_len.is_reserved());
        assert!(s.subslot2.frame_len.is_reserved());
    }

    // ---- Random access state machine (cl. 23.5.1.4.5–.9) --------------------

    /// Deterministic RNG for tests: returns queued subslot indices / choices.
    struct FakeRng {
        indices: std::collections::VecDeque<u16>,
        choices: std::collections::VecDeque<Subslot>,
    }
    impl FakeRng {
        fn new() -> Self {
            FakeRng {
                indices: Default::default(),
                choices: Default::default(),
            }
        }
        fn with_index(mut self, i: u16) -> Self {
            self.indices.push_back(i);
            self
        }
        fn with_choice(mut self, c: Subslot) -> Self {
            self.choices.push_back(c);
            self
        }
    }
    impl RaRng for FakeRng {
        fn choose_subslot_index(&mut self, frame_length: u16) -> u16 {
            self.indices.pop_front().unwrap_or(1).min(frame_length)
        }
        fn choose_one_of_two(&mut self) -> Subslot {
            self.choices.pop_front().unwrap_or(Subslot::One)
        }
    }

    fn params(imm: u8, wt: u8, nu: u8) -> RandomAccessParams {
        RandomAccessParams {
            imm,
            wt,
            nu,
            fl_factor: false,
            ts_ptr: 0,
            min_pdu_prio: 0,
            subscriber_class: None,
            gssi: None,
        }
    }

    fn t(ts: u8, f: u8) -> TdmaTime {
        TdmaTime { t: ts, f, m: 1, h: 0 }
    }

    /// A slot whose subslot 1 is an ongoing-frame opportunity for code A.
    fn ongoing_a() -> SlotAccessAssign {
        let s = SubslotAccess {
            access_code: Some(AccessCode::A),
            frame_len: BaseFrameLength::OngoingFrame,
        };
        SlotAccessAssign { subslot1: s, subslot2: s }
    }

    /// A slot whose subslot 1 is a frame marker for code A with the given base
    /// length and subslot 2 is an ongoing frame (cl. 23.5.1.4.2 point e).
    fn marker_a(base: u16) -> SlotAccessAssign {
        let raw = match base {
            1 => 0b0011,
            2 => 0b0100,
            3 => 0b0101,
            4 => 0b0110,
            _ => panic!("unsupported base"),
        };
        SlotAccessAssign {
            subslot1: SubslotAccess {
                access_code: Some(AccessCode::A),
                frame_len: BaseFrameLength::from_raw(raw),
            },
            subslot2: SubslotAccess {
                access_code: Some(AccessCode::A),
                frame_len: BaseFrameLength::OngoingFrame,
            },
        }
    }

    #[test]
    fn test_ra_imm15_first_valid_opportunity() {
        // IMM == 15: transmit in the first valid opportunity (cl. 23.5.1.4.5).
        let mut ra = MsRandomAccess::new();
        let p = params(15, 4, 3);
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        assert!(ra.is_active());
        let mut rng = FakeRng::new();
        // Subslot 1 only is an opportunity → transmit in subslot 1 at DL+2.
        let a = ra.poll_downlink_slot(t(1, 1), &ongoing_a(), true, &p, &mut rng);
        assert_eq!(
            a,
            Some(RaAction::Transmit {
                ul_time: t(1, 1).add_timeslots(2),
                subslot: Subslot::One,
            })
        );
    }

    #[test]
    fn test_ra_imm15_both_subslots_uses_rng() {
        let mut ra = MsRandomAccess::new();
        let p = params(15, 4, 3);
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        let both = ongoing_a(); // both subslots are opportunities for A
        let mut rng = FakeRng::new().with_choice(Subslot::Two);
        let a = ra.poll_downlink_slot(t(1, 1), &both, true, &p, &mut rng);
        match a {
            Some(RaAction::Transmit { subslot, .. }) => assert_eq!(subslot, Subslot::Two),
            other => panic!("expected transmit, got {other:?}"),
        }
    }

    #[test]
    fn test_ra_invalid_ul_slot_no_transmit() {
        // ts_pointer mismatch (ul_slot_valid == false) must not transmit
        // (cl. 23.5.1.4.7 point a).
        let mut ra = MsRandomAccess::new();
        let p = params(15, 4, 3);
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        let mut rng = FakeRng::new();
        assert_eq!(ra.poll_downlink_slot(t(1, 1), &ongoing_a(), false, &p, &mut rng), None);
    }

    #[test]
    fn test_ra_imm0_uses_new_access_frame() {
        // IMM == 0 always randomizes: no first-try, wait for a frame marker then
        // count to the chosen subslot (cl. 23.5.1.4.5 → .4.6).
        let mut ra = MsRandomAccess::new();
        let p = params(0, 4, 3);
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        // Choose subslot 2 within a 2-subslot frame.
        let mut rng = FakeRng::new().with_index(2);

        // An ongoing-frame slot before any marker is ignored.
        assert_eq!(ra.poll_downlink_slot(t(1, 1), &ongoing_a(), true, &p, &mut rng), None);

        // Marker slot: subslot 1 (marker) is count #1; subslot 2 (ongoing) is
        // count #2 == chosen → transmit in subslot 2.
        let a = ra.poll_downlink_slot(t(1, 2), &marker_a(2), true, &p, &mut rng);
        assert_eq!(
            a,
            Some(RaAction::Transmit {
                ul_time: t(1, 2).add_timeslots(2),
                subslot: Subslot::Two,
            })
        );
    }

    #[test]
    fn test_ra_new_frame_counts_across_slots() {
        // chosen == 3 within a base-4 frame: marker subslot (#1), marker subslot2
        // ongoing (#2), then the next slot's subslot1 (#3) → transmit there
        // (cl. 23.5.1.4.7).
        let mut ra = MsRandomAccess::new();
        let p = params(0, 4, 3);
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        let mut rng = FakeRng::new().with_index(3);

        assert_eq!(ra.poll_downlink_slot(t(1, 1), &marker_a(4), true, &p, &mut rng), None);
        let a = ra.poll_downlink_slot(t(1, 2), &ongoing_a(), true, &p, &mut rng);
        assert_eq!(
            a,
            Some(RaAction::Transmit {
                ul_time: t(1, 2).add_timeslots(2),
                subslot: Subslot::One,
            })
        );
    }

    #[test]
    fn test_ra_success_on_mac_resource() {
        let mut ra = MsRandomAccess::new();
        let p = params(15, 4, 3);
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        let mut rng = FakeRng::new();
        ra.poll_downlink_slot(t(1, 1), &ongoing_a(), true, &p, &mut rng);
        // A matching MAC-RESOURCE with the random access flag set → success.
        assert_eq!(ra.on_mac_resource(true, true), Some(RaAction::Succeeded));
        assert!(!ra.is_active());
        // Non-matching or non-ack MAC-RESOURCE is ignored.
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        assert_eq!(ra.on_mac_resource(false, true), None);
        assert_eq!(ra.on_mac_resource(true, false), None);
        assert!(ra.is_active());
    }

    #[test]
    fn test_ra_retry_then_abandon_after_nu() {
        // Nu == 2: transmit, WT expires (retry), transmit again, WT expires →
        // abandon with MaxTransmissions (cl. 23.5.1.4.8/.4.9).
        let mut ra = MsRandomAccess::new();
        let p = params(15, 2, 2);
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, false).unwrap();
        let mut rng = FakeRng::new().with_index(1).with_index(1);

        // TX #1.
        assert!(matches!(
            ra.poll_downlink_slot(t(1, 1), &ongoing_a(), true, &p, &mut rng),
            Some(RaAction::Transmit { .. })
        ));
        // WT counts one opportunity per frame; wt == 2 → retry (no action yet).
        assert_eq!(ra.poll_downlink_slot(t(1, 2), &ongoing_a(), true, &p, &mut rng), None);
        assert_eq!(ra.poll_downlink_slot(t(1, 3), &ongoing_a(), true, &p, &mut rng), None);
        // Now awaiting a new frame marker → TX #2 on the marker slot.
        assert!(matches!(
            ra.poll_downlink_slot(t(1, 4), &marker_a(1), true, &p, &mut rng),
            Some(RaAction::Transmit { .. })
        ));
        // WT expires again; tx_count == Nu == 2 → abandon.
        assert_eq!(ra.poll_downlink_slot(t(1, 5), &ongoing_a(), true, &p, &mut rng), None);
        assert_eq!(
            ra.poll_downlink_slot(t(1, 6), &ongoing_a(), true, &p, &mut rng),
            Some(RaAction::Failed(RaFailure::MaxTransmissions))
        );
        assert!(!ra.is_active());
    }

    #[test]
    fn test_ra_emergency_doubles_transmissions() {
        // Emergency (priority 7) doubles the transmission limit to 2·Nu
        // (cl. 23.5.1.4.9) and bypasses the min-priority check (cl. 23.5.1.4.4).
        let mut ra = MsRandomAccess::new();
        let mut p = params(15, 1, 1);
        p.min_pdu_prio = 5; // would normally reject a low-priority PDU
        // Emergency code A: permitted despite min_pdu_prio.
        ra.initiate(t(1, 1), AccessCode::A, &p, 0, true).unwrap();
        let mut rng = FakeRng::new().with_index(1).with_index(1);
        // 2·Nu == 2 transmissions allowed. TX #1 in the first-try slot.
        assert!(matches!(
            ra.poll_downlink_slot(t(1, 1), &ongoing_a(), true, &p, &mut rng),
            Some(RaAction::Transmit { .. })
        ));
        // WT == 1 → the next frame expires the wait and triggers a retry (no
        // action; now awaiting a new frame marker).
        assert_eq!(ra.poll_downlink_slot(t(1, 2), &ongoing_a(), true, &p, &mut rng), None);
        // A later marker slot → TX #2.
        assert!(matches!(
            ra.poll_downlink_slot(t(1, 3), &marker_a(1), true, &p, &mut rng),
            Some(RaAction::Transmit { .. })
        ));
        // Second WT expiry → abandon (2 transmissions done).
        assert_eq!(
            ra.poll_downlink_slot(t(1, 4), &ongoing_a(), true, &p, &mut rng),
            Some(RaAction::Failed(RaFailure::MaxTransmissions))
        );
    }

    #[test]
    fn test_ra_initiate_rejects_unavailable_and_low_priority() {
        let mut ra = MsRandomAccess::new();
        // Nu == 0 → unavailable (cl. 23.5.1.4.1).
        assert_eq!(
            ra.initiate(t(1, 1), AccessCode::A, &params(15, 4, 0), 0, false),
            Err(RaFailure::NoValidAccessCode)
        );
        // PDU priority below the minimum, non-emergency → not permitted
        // (cl. 23.5.1.4.4).
        let mut p = params(15, 4, 3);
        p.min_pdu_prio = 4;
        assert_eq!(
            ra.initiate(t(1, 1), AccessCode::A, &p, 2, false),
            Err(RaFailure::AccessCodeNotPermitted)
        );
        assert!(!ra.is_active());
    }
}

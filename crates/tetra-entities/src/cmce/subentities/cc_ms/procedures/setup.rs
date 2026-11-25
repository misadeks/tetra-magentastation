use super::*;
use tetra_ms_core::cmce::CmceEvent;
impl CcMsSubentity {
    pub fn originate_group_call(
        &mut self,
        queue: &mut MessageQueue,
        called: u32,
        basic: BasicServiceInformation,
        request_tx: bool,
    ) -> bool {
        let Ok(basic) = adapter::basic_to_core(&basic) else { return false };
        self.dispatch(
            queue,
            CmceEvent::Originate {
                called: TetraAddress::new(called, SsiType::Gssi),
                basic,
                duplex: false,
                request_tx,
                hook: false,
                external: None,
            },
        )
    }
    pub fn originate_individual_call(
        &mut self,
        queue: &mut MessageQueue,
        called: u32,
        basic: BasicServiceInformation,
        duplex: bool,
        request_tx: bool,
    ) -> bool {
        let Ok(basic) = adapter::basic_to_core(&basic) else { return false };
        self.dispatch(
            queue,
            CmceEvent::Originate {
                called: TetraAddress::new(called, SsiType::Issi),
                basic,
                duplex,
                request_tx,
                hook: true,
                external: None,
            },
        )
    }
    pub fn originate_external_call(
        &mut self,
        queue: &mut MessageQueue,
        called: u32,
        digits: &str,
        basic: BasicServiceInformation,
        duplex: bool,
        request_tx: bool,
    ) -> Result<(), String> {
        let external =
            tetra_ms_core::cmce::encode_external_number(digits).map_err(|error| format!("invalid external number: {error:?}"))?;
        let basic = adapter::basic_to_core(&basic).map_err(|error| format!("invalid basic service: {error:?}"))?;
        if self.dispatch(
            queue,
            CmceEvent::Originate {
                called: TetraAddress::new(called, SsiType::Issi),
                basic,
                duplex,
                request_tx,
                hook: true,
                external: Some(external.view()),
            },
        ) {
            Ok(())
        } else {
            Err("CMCE origination rejected".to_owned())
        }
    }
    pub fn send_u_alert(&mut self, queue: &mut MessageQueue, id: u16, response: &tncc::TnccSetupResponse) -> bool {
        let basic = response
            .basic_service_information
            .as_ref()
            .and_then(|b| pdu_basic_from_tncc(b).ok())
            .and_then(|b| adapter::basic_to_core(&b).ok());
        self.dispatch(
            queue,
            CmceEvent::Alert {
                id,
                basic,
                duplex: response.simplex_duplex_selection.as_bool(),
            },
        )
    }
    pub fn connect_call(&mut self, queue: &mut MessageQueue, id: u16, duplex: bool, basic: BasicServiceInformation) -> bool {
        let Ok(basic) = adapter::basic_to_core(&basic) else { return false };
        self.dispatch(queue, CmceEvent::Connect { id, basic, duplex })
    }
}

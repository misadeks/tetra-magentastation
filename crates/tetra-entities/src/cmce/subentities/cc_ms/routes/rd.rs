use super::*;
impl CcMsSubentity {
    pub fn route_rd_deliver(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        let SapMsgInner::LcmcMleUnitdataInd(prim) = message.msg else {
            return;
        };
        let Ok(view) = prim.sdu.as_bit_view() else { return };
        let Ok(offset) = u16::try_from(prim.sdu.get_pos()) else { return };
        let Ok(view) = view.window(offset, view.len().saturating_sub(offset)) else {
            return;
        };
        self.dispatch(
            queue,
            tetra_ms_core::cmce::CmceEvent::Downlink {
                sdu: view,
                route: CallRoute {
                    main_address: prim.received_tetra_address,
                    handle: prim.handle,
                    endpoint_id: prim.endpoint_id,
                    link_id: prim.link_id,
                },
            },
        );
    }
}

// Historical tests construct downlink objects directly. Serialize them with
// the BS codecs so these fixtures now exercise the portable wire decoder too.
#[cfg(test)]
macro_rules! fixture {
    ($name:ident, $ty:ty) => {
        impl CcMsSubentity {
            pub(in crate::cmce::subentities::cc_ms) fn $name(&mut self, queue: &mut MessageQueue, pdu: $ty, route: CallRoute) {
                let mut bits = BitBuffer::new_autoexpand(256);
                pdu.to_bitbuf(&mut bits).expect("historical fixture");
                self.dispatch(
                    queue,
                    tetra_ms_core::cmce::CmceEvent::Downlink {
                        sdu: bits.as_bit_view().expect("fixture bits"),
                        route,
                    },
                );
            }
        }
    };
}
#[cfg(test)]
fixture!(rx_d_setup, DSetup);
#[cfg(test)]
fixture!(rx_d_info, DInfo);
#[cfg(test)]
fixture!(rx_d_connect_ack, DConnectAcknowledge);
#[cfg(test)]
fixture!(rx_d_tx_granted, DTxGranted);

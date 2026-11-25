use tetra_saps::tmv::enums::logical_chans::LogicalChannel;

pub use tetra_phy_core::errorcontrol_params::*;

/// Map application channel identities to the portable parameters.
pub fn get_params(lchan: LogicalChannel) -> &'static ErrorControlParams {
    match lchan {
        LogicalChannel::Bsch => &BSCH_PARAMS,
        LogicalChannel::SchHd | LogicalChannel::Stch | LogicalChannel::Bnch => &SCH_HD_PARAMS,
        LogicalChannel::Aach => &AACH_PARAMS,
        LogicalChannel::SchF => &SCH_F_PARAMS,
        LogicalChannel::SchHu => &SCH_HU_PARAMS,
        LogicalChannel::TchS => &TCH_S_PARAMS,
        LogicalChannel::Tch24 => unimplemented!(),
        LogicalChannel::Tch48 => unimplemented!(),
        LogicalChannel::Tch72 => unimplemented!(),
        LogicalChannel::Blch => unimplemented!(),
        LogicalChannel::Clch => unimplemented!(),
    }
}

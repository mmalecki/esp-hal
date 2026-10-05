//! Wi-Fi Fine Timing Measurement (FTM).
//!
//! FTM (IEEE 802.11mc) measures the round-trip time of frames exchanged between an *initiator*
//! and a *responder*, from which the distance between the two can be estimated.
//!
//! - To act as a responder, start an access point with
//!   [`AccessPointConfig::with_ftm_responder`](crate::wifi::ap::AccessPointConfig::with_ftm_responder).
//! - To act as an initiator, start Wi-Fi in station (or access point + station) mode and call
//!   [`WifiController::ftm_initiate_session_async`]. The station does not need to be connected to
//!   the responder.
//!
//! FTM is disabled in the Wi-Fi driver unless the `wifi_ftm_enable` configuration option is set
//! (`ESP_RADIO_CONFIG_WIFI_FTM_ENABLE=true`). `wifi_ftm_initiator_support` and
//! `wifi_ftm_responder_support`, both on by default, enable each role.

#[cfg(wifi_ftm_initiator_support)]
use procmacros::BuilderLite;

use super::event::{Collection, FineTimingMeasurementReportInfo};
#[cfg(wifi_ftm_initiator_support)]
use super::event::{self, EventInfo, WifiEvent};
#[cfg(any(wifi_ftm_initiator_support, wifi_ftm_responder_support))]
use super::{WifiController, WifiError, esp_wifi_result};
#[cfg(any(wifi_ftm_initiator_support, wifi_ftm_responder_support))]
use crate::sys::include;

/// Configuration of an FTM initiator session.
#[cfg(wifi_ftm_initiator_support)]
#[derive(BuilderLite, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct InitiatorConfig {
    /// MAC address (BSSID) of the FTM responder.
    pub(crate) responder_mac: [u8; 6],
    /// Primary channel of the FTM responder.
    ///
    /// If this differs from the channel the station is connected on, or the channel the local
    /// access point runs on, the driver falls back to a single burst in ASAP mode.
    pub(crate) channel: u8,
    /// Number of FTM frames requested, in bursts of 4 or 8.
    ///
    /// One of 0 (no preference), 8, 16, 24, 32 or 64.
    pub(crate) frame_count: u8,
    /// Requested period between FTM bursts, in units of 100 ms.
    ///
    /// Between 0 (no preference) and 100.
    pub(crate) burst_period: u16,
}

#[cfg(wifi_ftm_initiator_support)]
impl Default for InitiatorConfig {
    fn default() -> Self {
        Self {
            responder_mac: [0; 6],
            channel: 1,
            frame_count: 32,
            burst_period: 2,
        }
    }
}

#[cfg(wifi_ftm_initiator_support)]
impl InitiatorConfig {
    fn validate(&self) -> Result<(), WifiError> {
        // The driver checks the channel itself, against the regulatory domain.
        if !matches!(self.frame_count, 0 | 8 | 16 | 24 | 32 | 64) || self.burst_period > 100 {
            return Err(WifiError::InvalidArguments);
        }

        Ok(())
    }

    fn to_raw(self) -> include::wifi_ftm_initiator_cfg_t {
        include::wifi_ftm_initiator_cfg_t {
            resp_mac: self.responder_mac,
            channel: self.channel,
            frm_count: self.frame_count,
            burst_period: self.burst_period,
        }
    }
}

/// Outcome of an FTM session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Status {
    /// The FTM exchange succeeded.
    Success,
    /// The peer does not support FTM.
    Unsupported,
    /// The peer rejected the FTM configuration in the FTM request.
    ConfigurationRejected,
    /// The peer did not respond to FTM requests.
    NoResponse,
    /// Unknown error during the FTM exchange.
    Failed,
    /// The session did not result in any valid measurements.
    NoValidMeasurement,
    /// The session was ended by the user.
    UserTerminated,
    /// A status this version of `esp-radio` does not know about.
    Unknown(u32),
}

impl Status {
    #[cfg(wifi_ftm_initiator_support)]
    #[allow(non_upper_case_globals)]
    pub(crate) fn from_raw(raw: u32) -> Self {
        match raw {
            include::wifi_ftm_status_t_FTM_STATUS_SUCCESS => Self::Success,
            include::wifi_ftm_status_t_FTM_STATUS_UNSUPPORTED => Self::Unsupported,
            include::wifi_ftm_status_t_FTM_STATUS_CONF_REJECTED => Self::ConfigurationRejected,
            include::wifi_ftm_status_t_FTM_STATUS_NO_RESPONSE => Self::NoResponse,
            include::wifi_ftm_status_t_FTM_STATUS_FAIL => Self::Failed,
            include::wifi_ftm_status_t_FTM_STATUS_NO_VALID_MSMT => Self::NoValidMeasurement,
            include::wifi_ftm_status_t_FTM_STATUS_USER_TERM => Self::UserTerminated,
            other => Self::Unknown(other),
        }
    }
}

/// Result of an FTM initiator session.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub struct Report {
    /// MAC address of the FTM peer.
    pub peer_mac: [u8; 6],
    /// Outcome of the session. The other fields are only meaningful on [`Status::Success`].
    pub status: Status,
    /// Raw average round-trip time with the peer, in nanoseconds.
    pub rtt_raw_ns: u32,
    /// Estimated round-trip time with the peer, in nanoseconds.
    pub rtt_est_ns: u32,
    /// Estimated one-way distance to the peer, in centimeters.
    pub distance_cm: u32,
    /// The individual measurements the estimates were made from.
    pub entries: Collection<FineTimingMeasurementReportInfo>,
}

/// Ends the driver's initiator session if the future awaiting it is dropped.
#[cfg(wifi_ftm_initiator_support)]
struct EndSessionOnDrop;

#[cfg(wifi_ftm_initiator_support)]
impl EndSessionOnDrop {
    fn defuse(self) {
        core::mem::forget(self);
    }
}

#[cfg(wifi_ftm_initiator_support)]
impl Drop for EndSessionOnDrop {
    fn drop(&mut self) {
        unsafe { include::esp_wifi_ftm_end_session() };
    }
}

#[cfg(any(wifi_ftm_initiator_support, wifi_ftm_responder_support))]
impl WifiController<'_> {
    /// Runs an FTM session as initiator against the responder in `config`, and returns its
    /// report.
    ///
    /// Wi-Fi must be started in station or access point + station mode. The station does not
    /// need to be connected to the responder.
    ///
    /// A failed exchange is not an error: it is a [`Report`] whose [`Report::status`] says what
    /// went wrong. Errors are reserved for the driver refusing to start the session.
    #[cfg(wifi_ftm_initiator_support)]
    #[instability::unstable]
    pub async fn ftm_initiate_session_async(
        &mut self,
        config: &InitiatorConfig,
    ) -> Result<Report, WifiError> {
        config.validate()?;

        event::enable_wifi_events(WifiEvent::FineTimingMeasurementReport.into());

        // Subscribe before starting the session, so the report cannot be missed.
        let mut subscriber = super::EVENT_CHANNEL
            .subscriber()
            .expect("Unable to subscribe to events - consider increasing the internal event channel subscriber count");

        let mut raw = config.to_raw();
        esp_wifi_result!(unsafe { include::esp_wifi_ftm_initiate_session(&mut raw) })?;

        let guard = EndSessionOnDrop;

        loop {
            if let EventInfo::FineTimingMeasurementReport {
                peer_mac,
                status,
                rtt_raw,
                rtt_est,
                dist_est,
                entries,
            } = subscriber.next_message_pure().await
                && peer_mac == config.responder_mac
            {
                guard.defuse();

                return Ok(Report {
                    peer_mac,
                    status: Status::from_raw(status),
                    rtt_raw_ns: rtt_raw,
                    rtt_est_ns: rtt_est,
                    distance_cm: dist_est,
                    entries,
                });
            }
        }
    }

    /// Ends the ongoing FTM initiator session, if any.
    #[cfg(wifi_ftm_initiator_support)]
    #[instability::unstable]
    pub fn ftm_end_session(&mut self) -> Result<(), WifiError> {
        esp_wifi_result!(unsafe { include::esp_wifi_ftm_end_session() })
    }

    /// Sets an offset, in centimeters, which the FTM responder adds to the time of departure of
    /// its measurement frames.
    ///
    /// Use this in access point mode, before performing FTM as a responder, to calibrate out a
    /// constant error in the distance an initiator measures.
    #[cfg(wifi_ftm_responder_support)]
    #[instability::unstable]
    pub fn ftm_responder_set_offset(&mut self, offset_cm: i16) -> Result<(), WifiError> {
        esp_wifi_result!(unsafe { include::esp_wifi_ftm_resp_set_offset(offset_cm) })
    }
}

use anyhow::{anyhow, Result};

use super::capabilities::{AGC_SETPOINT_RANGE_DBFS, IF_GAIN_REDUCTION_RANGE_DB, MAX_LNA_STATE};
use super::{rates, Capabilities, Command, Event, ReceiverState};

#[derive(Debug)]
pub enum CommandResult {
    Event(Event),
    /// Several events, to be applied in order.
    Events(Vec<Event>),
    State(ReceiverState),
    Capabilities(Capabilities),
}

#[derive(Debug)]
pub struct Receiver {
    state: ReceiverState,
    capabilities: Capabilities,
}

impl Receiver {
    pub fn new(state: ReceiverState, capabilities: Capabilities) -> Self {
        Self {
            state,
            capabilities,
        }
    }

    pub fn state(&self) -> &ReceiverState {
        &self.state
    }

    /// Used by the outputs that need to know the receiver's limits (hamlib, ...).
    #[allow(dead_code)]
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn handle_command(&mut self, command: Command) -> Result<CommandResult> {
        match command {
            Command::GetState => Ok(CommandResult::State(self.state.clone())),

            Command::GetCapabilities => Ok(CommandResult::Capabilities(self.capabilities.clone())),

            Command::SetFrequency(frequency_hz) => {
                if frequency_hz < self.capabilities.frequency_min_hz
                    || frequency_hz > self.capabilities.frequency_max_hz
                {
                    return Err(anyhow!(
                        "Frequency out of range: {} Hz ({} - {} Hz)",
                        frequency_hz,
                        self.capabilities.frequency_min_hz,
                        self.capabilities.frequency_max_hz
                    ));
                }

                self.state.frequency_hz = frequency_hz;

                Ok(CommandResult::Event(Event::FrequencyChanged(frequency_hz)))
            }

            Command::SetMode(mode) => {
                if !self.capabilities.modes.contains(&mode) {
                    return Err(anyhow!("Unsupported mode: {:?}", mode));
                }

                self.state.mode = mode;

                Ok(CommandResult::Event(Event::ModeChanged(mode)))
            }

            Command::SetBandwidth(bandwidth_hz) => {
                if !self.capabilities.bandwidths_hz.contains(&bandwidth_hz) {
                    return Err(anyhow!("Unsupported bandwidth: {} Hz", bandwidth_hz));
                }

                self.state.bandwidth_hz = bandwidth_hz;

                Ok(CommandResult::Event(Event::BandwidthChanged(bandwidth_hz)))
            }

            Command::SetIfType(if_type) => {
                if !self.capabilities.if_types.contains(&if_type) {
                    return Err(anyhow!("Unsupported IF type: {:?}", if_type));
                }

                self.state.if_type = if_type;

                Ok(CommandResult::Event(Event::IfTypeChanged(if_type)))
            }

            Command::SetLoMode(lo_mode) => {
                log::debug!("SetLoMode received: {:?}", lo_mode);
                if !self.capabilities.lo_modes.contains(&lo_mode) {
                    return Err(anyhow!("Unsupported LO mode: {:?}", lo_mode));
                }

                self.state.lo_mode = lo_mode;

                Ok(CommandResult::Event(Event::LoModeChanged(lo_mode)))
            }

            Command::SetSampleRate(sample_rate) => {
                if rates::plan(sample_rate).is_none() {
                    return Err(anyhow!(
                        "Sample rate {} Hz outside the supported range ({}–{} Hz)",
                        sample_rate,
                        self.capabilities.sample_rate_min_hz,
                        self.capabilities.sample_rate_max_hz
                    ));
                }

                self.state.sample_rate = sample_rate;

                // The analog filter follows the rate (widest width that fits),
                // like SDRplay's rsp_tcp. A client that wants another width
                // sends it afterwards (rtl_tcp command 0x40).
                let bandwidth_hz = rates::default_bandwidth_hz(sample_rate);
                self.state.bandwidth_hz = bandwidth_hz;

                Ok(CommandResult::Events(vec![
                    Event::SampleRateChanged(sample_rate),
                    Event::BandwidthChanged(bandwidth_hz),
                ]))
            }

            Command::SetGain(gain) => {
                if gain < self.capabilities.gain_min_db || gain > self.capabilities.gain_max_db {
                    return Err(anyhow!(
                        "Gain out of range: {:.2} dB ({:.2} - {:.2} dB)",
                        gain,
                        self.capabilities.gain_min_db,
                        self.capabilities.gain_max_db
                    ));
                }

                self.state.gain = gain;

                Ok(CommandResult::Event(Event::GainChanged(gain)))
            }

            Command::SetGainIndex(index) => {
                if index >= self.capabilities.gain_steps {
                    return Err(anyhow!(
                        "Gain step out of range: {} (0 - {})",
                        index,
                        self.capabilities.gain_steps - 1
                    ));
                }

                self.state.gain_index = index;

                Ok(CommandResult::Event(Event::GainIndexChanged(index)))
            }

            Command::SetLnaState(lna_state) => {
                if lna_state > MAX_LNA_STATE {
                    return Err(anyhow!(
                        "LNA state out of range: {} (0 - {})",
                        lna_state,
                        MAX_LNA_STATE
                    ));
                }

                Ok(CommandResult::Event(Event::LnaStateChanged(lna_state)))
            }

            Command::SetIfGainReduction(gr_db) => {
                let (min, max) = IF_GAIN_REDUCTION_RANGE_DB;

                if !(min..=max).contains(&gr_db) {
                    return Err(anyhow!(
                        "IF gain reduction out of range: {} dB ({} - {} dB)",
                        gr_db,
                        min,
                        max
                    ));
                }

                Ok(CommandResult::Event(Event::IfGainReductionChanged(gr_db)))
            }

            Command::SetAgcSetpoint(setpoint_dbfs) => {
                let (min, max) = AGC_SETPOINT_RANGE_DBFS;

                if !(min..=max).contains(&setpoint_dbfs) {
                    return Err(anyhow!(
                        "AGC set-point out of range: {} dBFS ({} - {} dBFS)",
                        setpoint_dbfs,
                        min,
                        max
                    ));
                }

                Ok(CommandResult::Event(Event::AgcSetpointChanged(
                    setpoint_dbfs,
                )))
            }

            Command::SetGainMode(gain_mode) => {
                self.state.gain_mode = gain_mode;

                Ok(CommandResult::Event(Event::GainModeChanged(gain_mode)))
            }

            Command::SetAgc(agc) => {
                self.state.agc = agc;

                Ok(CommandResult::Event(Event::AgcChanged(agc)))
            }

            Command::SetAntenna(antenna) => {
                if !self.capabilities.antennas.contains(&antenna) {
                    return Err(anyhow!("Unsupported antenna: {}", antenna));
                }

                self.state.antenna = Some(antenna.clone());

                Ok(CommandResult::Event(Event::AntennaChanged(antenna)))
            }

            Command::SetPpm(ppm) => {
                self.state.ppm = ppm;

                Ok(CommandResult::Event(Event::PpmChanged(ppm)))
            }

            Command::SetBiasTee(enabled) => {
                if !self.capabilities.bias_tee {
                    return Err(anyhow!("Bias-T is not supported by this device"));
                }

                self.state.bias_tee = enabled;

                Ok(CommandResult::Event(Event::BiasTeeChanged(enabled)))
            }

            Command::SetRfNotch(enabled) => {
                if !self.capabilities.rf_notch {
                    return Err(anyhow!("RF notch is not supported by this device"));
                }

                self.state.rf_notch = enabled;

                Ok(CommandResult::Event(Event::RfNotchChanged(enabled)))
            }

            Command::SetDabNotch(enabled) => {
                if !self.capabilities.dab_notch {
                    return Err(anyhow!("DAB notch is not supported by this device"));
                }

                self.state.dab_notch = enabled;

                Ok(CommandResult::Event(Event::DabNotchChanged(enabled)))
            }

            Command::StartIq => {
                if !self.capabilities.iq {
                    return Err(anyhow!("IQ streaming is not supported by this device"));
                }

                if self.state.streaming {
                    return Err(anyhow!("IQ streaming is already active"));
                }

                self.state.streaming = true;

                Ok(CommandResult::Event(Event::IqStarted))
            }

            Command::StopIq => {
                if !self.state.streaming {
                    return Err(anyhow!("IQ streaming is not active"));
                }

                self.state.streaming = false;

                Ok(CommandResult::Event(Event::IqStopped))
            }
        }
    }
}

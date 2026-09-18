use anyhow::{anyhow, Result};

use super::{Capabilities, Command, Event, ReceiverState};

#[derive(Debug)]
pub enum CommandResult {
    Event(Event),
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
                        "Fréquence hors limites : {} Hz ({} - {} Hz)",
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
                    return Err(anyhow!("Mode non supporté : {:?}", mode));
                }

                self.state.mode = mode;

                Ok(CommandResult::Event(Event::ModeChanged))
            }

            Command::SetBandwidth(bandwidth_hz) => {
                if !self.capabilities.bandwidths_hz.contains(&bandwidth_hz) {
                    return Err(anyhow!(
                        "Bande passante non supportée : {} Hz",
                        bandwidth_hz
                    ));
                }

                self.state.bandwidth_hz = bandwidth_hz;

                Ok(CommandResult::Event(Event::BandwidthChanged(bandwidth_hz)))
            }

            Command::SetSampleRate(sample_rate) => {
                if !self.capabilities.sample_rates.contains(&sample_rate) {
                    return Err(anyhow!("Sample rate non supporté : {} Hz", sample_rate));
                }

                self.state.sample_rate = sample_rate;

                Ok(CommandResult::Event(Event::SampleRateChanged(sample_rate)))
            }

            Command::SetGain(gain) => {
                if gain < self.capabilities.gain_min_db || gain > self.capabilities.gain_max_db {
                    return Err(anyhow!(
                        "Gain hors limites : {:.2} dB ({:.2} - {:.2} dB)",
                        gain,
                        self.capabilities.gain_min_db,
                        self.capabilities.gain_max_db
                    ));
                }

                self.state.gain = gain;

                Ok(CommandResult::Event(Event::GainChanged(gain)))
            }

            Command::SetGainMode(gain_mode) => {
                self.state.gain_mode = gain_mode;

                Ok(CommandResult::Event(Event::GainModeChanged))
            }

            Command::SetAgc(agc) => {
                self.state.agc = agc;

                Ok(CommandResult::Event(Event::AgcChanged(agc)))
            }

            Command::SetAntenna(antenna) => {
                if !self.capabilities.antennas.contains(&antenna) {
                    return Err(anyhow!("Antenne non supportée : {}", antenna));
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
                    return Err(anyhow!(
                        "Le bias-tee n'est pas supporté par ce périphérique"
                    ));
                }

                self.state.bias_tee = enabled;

                Ok(CommandResult::Event(Event::BiasTeeChanged(enabled)))
            }

            Command::SetRfNotch(enabled) => {
                if !self.capabilities.rf_notch {
                    return Err(anyhow!(
                        "Le RF notch n'est pas supporté par ce périphérique"
                    ));
                }

                self.state.rf_notch = enabled;

                Ok(CommandResult::Event(Event::RfNotchChanged(enabled)))
            }

            Command::SetDabNotch(enabled) => {
                if !self.capabilities.dab_notch {
                    return Err(anyhow!(
                        "Le DAB notch n'est pas supporté par ce périphérique"
                    ));
                }

                self.state.dab_notch = enabled;

                Ok(CommandResult::Event(Event::DabNotchChanged(enabled)))
            }

            Command::StartIq => {
                if !self.capabilities.iq {
                    return Err(anyhow!("Le flux IQ n'est pas supporté par ce périphérique"));
                }

                if self.state.streaming {
                    return Err(anyhow!("Le flux IQ est déjà actif"));
                }

                self.state.streaming = true;

                Ok(CommandResult::Event(Event::IqStarted))
            }

            Command::StopIq => {
                if !self.state.streaming {
                    return Err(anyhow!("Le flux IQ n'est pas actif"));
                }

                self.state.streaming = false;

                Ok(CommandResult::Event(Event::IqStopped))
            }
        }
    }
}

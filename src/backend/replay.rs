//! Memory of the settings applied to a receiver.
//!
//! When the receiver is unplugged, or its service restarts, it comes back with
//! its default settings. The backend then replays what was applied before, so
//! that clients, which still believe the frequency, gain and sample rate they
//! chose are in effect, find the receiver as they left it.
//!
//! Only the **last** event of each kind is kept, in the order those last events
//! happened: replaying them leads to the same final state as the full history
//! (for example, a sample rate always comes before the bandwidth that followed
//! it).

use crate::core::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    SampleRate,
    Bandwidth,
    IfType,
    LoMode,
    Frequency,
    /// `GainChanged` and `GainIndexChanged`: two ways of choosing a gain step.
    GainStep,
    LnaState,
    IfGainReduction,
    GainMode,
    AgcSetpoint,
    BiasTee,
    RfNotch,
    DabNotch,
    Ppm,
    /// `IqStarted` / `IqStopped`.
    Stream,
}

fn kind(event: &Event) -> Option<Kind> {
    Some(match event {
        Event::SampleRateChanged(_) => Kind::SampleRate,
        Event::BandwidthChanged(_) => Kind::Bandwidth,
        Event::IfTypeChanged(_) => Kind::IfType,
        Event::LoModeChanged(_) => Kind::LoMode,
        Event::FrequencyChanged(_) => Kind::Frequency,
        Event::GainChanged(_) | Event::GainIndexChanged(_) => Kind::GainStep,
        Event::LnaStateChanged(_) => Kind::LnaState,
        Event::IfGainReductionChanged(_) => Kind::IfGainReduction,
        Event::GainModeChanged(_) => Kind::GainMode,
        Event::AgcSetpointChanged(_) => Kind::AgcSetpoint,
        Event::BiasTeeChanged(_) => Kind::BiasTee,
        Event::RfNotchChanged(_) => Kind::RfNotch,
        Event::DabNotchChanged(_) => Kind::DabNotch,
        Event::PpmChanged(_) => Kind::Ppm,
        Event::IqStarted | Event::IqStopped => Kind::Stream,
        _ => return None,
    })
}

#[derive(Debug, Default)]
pub struct DesiredSettings {
    events: Vec<(Kind, Event)>,
}

impl DesiredSettings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remembers an event that was applied successfully. Events that are not
    /// receiver settings are ignored.
    pub fn record(&mut self, event: &Event) {
        let Some(kind) = kind(event) else {
            return;
        };

        self.events.retain(|(existing, _)| *existing != kind);
        self.events.push((kind, event.clone()));
    }

    /// The settings to re-apply, in order (the stream state excluded).
    pub fn settings(&self) -> Vec<Event> {
        self.events
            .iter()
            .filter(|(kind, _)| *kind != Kind::Stream)
            .map(|(_, event)| event.clone())
            .collect()
    }

    /// Whether the IQ stream was running.
    pub fn stream_wanted(&self) -> bool {
        self.events
            .iter()
            .any(|(kind, event)| *kind == Kind::Stream && matches!(event, Event::IqStarted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::GainMode;

    fn names(settings: &[Event]) -> Vec<String> {
        settings
            .iter()
            .map(|e| format!("{:?}", e).split('(').next().unwrap().to_string())
            .collect()
    }

    #[test]
    fn only_the_last_event_of_each_kind_is_kept() {
        let mut desired = DesiredSettings::new();

        desired.record(&Event::FrequencyChanged(100_000_000));
        desired.record(&Event::FrequencyChanged(195_936_000));
        desired.record(&Event::FrequencyChanged(1_090_000_000));

        let settings = desired.settings();
        assert_eq!(settings.len(), 1);
        assert!(matches!(
            settings[0],
            Event::FrequencyChanged(1_090_000_000)
        ));
    }

    #[test]
    fn order_follows_the_last_occurrences() {
        let mut desired = DesiredSettings::new();

        desired.record(&Event::SampleRateChanged(2_048_000));
        desired.record(&Event::BandwidthChanged(1_536_000));
        desired.record(&Event::FrequencyChanged(195_936_000));
        // The client asks for another rate: the core then also sends the
        // default bandwidth for it, so the rate stays before the bandwidth.
        desired.record(&Event::SampleRateChanged(1_024_000));
        desired.record(&Event::BandwidthChanged(600_000));

        assert_eq!(
            names(&desired.settings()),
            vec!["FrequencyChanged", "SampleRateChanged", "BandwidthChanged"]
        );
    }

    #[test]
    fn gain_step_events_replace_each_other() {
        let mut desired = DesiredSettings::new();

        desired.record(&Event::GainIndexChanged(14));
        desired.record(&Event::GainChanged(25.0));
        desired.record(&Event::GainIndexChanged(20));

        let settings = desired.settings();
        assert_eq!(settings.len(), 1);
        assert!(matches!(settings[0], Event::GainIndexChanged(20)));
    }

    #[test]
    fn direct_gain_controls_are_kept_after_the_step() {
        let mut desired = DesiredSettings::new();

        desired.record(&Event::GainIndexChanged(14));
        desired.record(&Event::LnaStateChanged(5));
        desired.record(&Event::IfGainReductionChanged(44));
        desired.record(&Event::GainModeChanged(GainMode::Manual));

        assert_eq!(
            names(&desired.settings()),
            vec![
                "GainIndexChanged",
                "LnaStateChanged",
                "IfGainReductionChanged",
                "GainModeChanged"
            ]
        );
    }

    #[test]
    fn stream_state_is_tracked_but_not_replayed_as_a_setting() {
        let mut desired = DesiredSettings::new();
        assert!(!desired.stream_wanted());

        desired.record(&Event::FrequencyChanged(100_000_000));
        desired.record(&Event::IqStarted);
        assert!(desired.stream_wanted());
        assert_eq!(desired.settings().len(), 1);

        desired.record(&Event::IqStopped);
        assert!(!desired.stream_wanted());
    }

    #[test]
    fn events_that_are_not_settings_are_ignored() {
        let mut desired = DesiredSettings::new();

        desired.record(&Event::Overflow);
        desired.record(&Event::Error("x".to_string()));
        desired.record(&Event::DeviceConnected);
        desired.record(&Event::StateChanged);

        assert!(desired.settings().is_empty());
    }

    #[test]
    fn every_setting_kind_is_recorded() {
        let mut desired = DesiredSettings::new();
        let events = [
            Event::SampleRateChanged(2_000_000),
            Event::BandwidthChanged(1_536_000),
            Event::FrequencyChanged(100_000_000),
            Event::GainIndexChanged(14),
            Event::LnaStateChanged(3),
            Event::IfGainReductionChanged(40),
            Event::GainModeChanged(GainMode::Automatic),
            Event::AgcSetpointChanged(-40),
            Event::BiasTeeChanged(true),
            Event::RfNotchChanged(true),
            Event::DabNotchChanged(false),
            Event::PpmChanged(1.5),
        ];

        for event in &events {
            desired.record(event);
        }

        assert_eq!(desired.settings().len(), events.len());
    }
}

use super::DomainEvent;

#[derive(Debug, PartialEq)]
pub enum Topic {
    System,
    TextInput,
    KeyInput,
    KeyOutput,
    Stats,
    Monitoring,
    Telemetry,
}

impl From<&DomainEvent> for Topic {
    fn from(value: &DomainEvent) -> Self {
        use DomainEvent::*;
        use Topic::*;
        match value {
            KeyPress(..) => KeyInput,
            KeyRelease(..) => KeyInput,
            HidReport(_) => KeyOutput,
            SendText(_) => TextInput,
            SendFile(..) => TextInput,
            TextSent => Monitoring,
            CurrentStats(_) => Stats,

            ModeChange(_) => System,
            Exit => System,
            Sleep => System,
            WakeUp => System,

            ReportSent() => Telemetry,
        }
    }
}
